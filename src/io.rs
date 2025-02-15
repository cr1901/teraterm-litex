/*! I/O hooks. */

use core::slice;
use std::ffi::c_void;
use std::fs::File;
use std::{io, ptr};

use crate::sfl::SflLoader;

use super::sfl::{Resp, MAGIC_RESPONSE};
use super::state::{Activity, State, TTX_LITEX_STATE};
use super::tt;
use super::Error;
use log::*;

use windows::Win32::System::IO::OVERLAPPED;

#[allow(unused)]
unsafe extern "C" fn our_p_read_file(
    fh: *mut c_void,
    buff: *mut c_void,
    len: u32,
    read_bytes: *mut u32,
    wol: *mut OVERLAPPED,
) -> i32 {
    trace!(target: "our_p_read_file", "Entered");

    let mut num_read = 0;
    TTX_LITEX_STATE
        .with_borrow_mut(|mut s| {
            s.orig_readfile
            .ok_or(Error::WasEmpty("PReadFile"))
            .and_then(|read_file| {
                trace!(target: "our_p_read_file", "Running original PReadFile at {:?}.", read_file);
                num_read = read_file(fh, buff, len, read_bytes, wol);

                if *read_bytes == 0 {
                    return Ok(num_read);
                }

                let chunk = slice::from_raw_parts(buff as *const u8, *read_bytes as usize);
                drive_sfl(&mut s, chunk)?;
                Ok(num_read)
            })
        })
        .inspect_err(|e| error!(target: "our_p_read_file", "Failed to drive SFL FSM: {}", e))
        .unwrap_or(num_read)
}

fn drive_sfl(s: &mut State, chunk: &[u8]) -> Result<(), Error> {
    // todo!()
    match &mut s.activity {
        Activity::Inactive => {}
        Activity::LookForMagic => {
            if !s.matcher.look_for_match(chunk) {
                return Ok(());
            }

            s.matcher.reset();
            info!(target: "drive_sfl", "Found magic string.");

            let mut loader = s
                .filename
                .as_ref()
                .ok_or(Error::WasEmpty("filename"))
                .and_then(|filename| File::open(filename).map_err(|e| Error::FileIoError(e)))
                .and_then(|fp| {
                    if fp.metadata().map_err(|e| Error::FileIoError(e))?.len() == 0 {
                        return Err(Error::FileIoError(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "input file was empty",
                        )));
                    } else {
                        Ok(fp)
                    }
                })
                .map(|fp| SflLoader::new(fp, s.addr))?;

            inject_output(s, &MAGIC_RESPONSE)?;
            let frame = loader
                .encode_data_frame(0)
                .map_err(|e| Error::FileIoError(e))?
                .unwrap();
            trace!("first: {:02X?}", frame);
            inject_output(s, frame.as_bytes())?;
            // Mutably borrowed twice???
            // fn next_data_frame(s: &mut State) -> Result<Option<Box<Frame>>, Error> {
            //     todo!()
            // }
            // inject_output(s, next_data_frame(s)?.unwrap().as_bytes())?;

            s.sfl_loader = Some(loader);
            s.activity = Activity::WaitResp;
            s.curr_frame = Some(frame);
            s.last_frame_sent = Some(0);

            return Ok(());
        }
        Activity::WaitResp => {
            match Resp::try_from(chunk[0]).map_err(|_| Error::UnexpectedResponse(chunk[0]))? {
                Resp::Success => {
                    s.last_frame_acked = s.last_frame_sent;
                    let next_frame = s.last_frame_sent.unwrap();

                    match s
                        .sfl_loader
                        .as_mut()
                        .unwrap()
                        .encode_data_frame(next_frame)
                        .map_err(|e| Error::FileIoError(e))?
                    {
                        Some(frame) => {
                            trace!("next: {:X?}", frame);
                            inject_output(s, frame.as_bytes())?;
                            s.curr_frame = Some(frame);
                            s.last_frame_sent = Some(next_frame + 1);
                        }
                        None => {
                            let frame = s.sfl_loader.as_mut().unwrap().encode_boot_frame(s.addr);
                            trace!("final: {:X?}", frame);
                            inject_output(s, frame.as_bytes())?;
                            s.curr_frame = Some(frame);
                            s.activity = Activity::WaitFinalResp;
                        }
                    }
                }
                err @ (Resp::CrcError | Resp::Unknown | Resp::AckError) => {
                    info!(target: "drive_sfl", "SFL Error: {}, resending current", err);
                    let frame = s.curr_frame.take().unwrap();
                    trace!("resend: {:X?}", frame);
                    inject_output(s, frame.as_bytes())?;
                    s.curr_frame = Some(frame);

                    return Ok(());
                }
            }
        }
        Activity::WaitFinalResp => {
            match Resp::try_from(chunk[0]).map_err(|_| Error::UnexpectedResponse(chunk[0]))? {
                Resp::Success => {
                    s.last_frame_acked = None;
                    s.last_frame_sent = None;
                    s.activity = Activity::LookForMagic;
                }
                err @ (Resp::CrcError | Resp::Unknown | Resp::AckError) => {
                    info!(target: "drive_sfl", "SFL Error: {}, resending current", err);
                    let frame = s.curr_frame.take().unwrap();
                    trace!("resent_final: {:X?}", frame);
                    inject_output(s, frame.as_bytes())?;
                    s.curr_frame = Some(frame);

                    return Ok(());
                }
            }
        }
    }

    return Ok(());
}

fn inject_output(s: &mut State, buf: &[u8]) -> Result<(), Error> {
    // SAFETY: Assumes TeraTerm passed us valid pointers. We can't use
    // &mut because I have no idea whether we truly have exclusive access.

    let out_buff = unsafe { &raw mut (*s.cv).OutBuff };
    let len = unsafe { (*s.cv).OutBuffCount } as u32;
    let ptr = unsafe { (*s.cv).OutPtr } as u32;

    if (ptr + len) >= tt::OutBuffSize {
        return Err(Error::OutBuffOutOfBounds(ptr + len));
    }

    let max_out_size = tt::OutBuffSize - len;
    if buf.len() > max_out_size as usize {
        return Err(Error::OutBuffFull {
            need: buf.len() as u32,
            actual: max_out_size,
        });
    }

    let src = unsafe { &raw const (*out_buff)[ptr as usize] };
    let dst = unsafe { &raw mut (*out_buff)[0] };

    // SAFETY:
    // * We checked that src is in bounds.
    // * dst must be in bounds because it's the beginning of out_buff.
    // * u8 is Copy.
    unsafe { ptr::copy(src, dst, len as usize) };

    let our_buf_ptr = buf.as_ptr();
    // SAFETY: If the initial bounds check at a non-zero offset passed, then
    // so will this one. We've already got UB problems if len > ISIZE_MAX.
    let our_dst = unsafe { dst.offset(len as isize) };

    // SAFETY:
    // * Our src is from Rust.
    // * dst must be in bounds from previous checks.
    // * u8 is Copy.
    unsafe { ptr::copy(our_buf_ptr, our_dst, buf.len()) };

    // OutBuff is NOT circular; ptr is "next value to be written", and
    // cnt is "num of values left to write". Once cnt becomes 0, ptr also
    // gets reset to 0. (See "CommSend", which is the immediate parent
    // function of PWriteFile).
    unsafe {
        *(&raw mut (*s.cv).OutBuffCount) = (buf.len() + len as usize) as i32;
        *(&raw mut (*s.cv).OutPtr) = 0;
    }

    Ok(())
}

pub unsafe extern "C" fn ttx_open_file(hooks: *mut tt::TTXFileHooks) {
    TTX_LITEX_STATE.with_borrow_mut(|s| {
        // SAFETY: Assumes TeraTerm passed us valid pointers.
        s.orig_readfile = *(*hooks).PReadFile;
        // s.orig_writefile = *(*hooks).PWriteFile;
        *(*hooks).PReadFile = Some(our_p_read_file);
        // *(*hooks).PWriteFile = Some(our_p_write_file);

        trace!(target: "TTXOpenFile", "s.orig_readfile <= {:?} ({:?})", &raw const s.orig_readfile, s.orig_readfile);
        // trace!(target: "TTXOpenFile", "s.orig_writefile <= {:?} ({:?})", &raw const s.orig_writefile, s.orig_writefile);
        trace!(target: "TTXOpenFile", "*(*hooks).PReadFile <= {:?}", our_p_read_file as * const ());
        // trace!(target: "TTXOpenFile", "*(*hooks).PWriteFile <= {:?}", our_p_write_file as * const ());
    });
}

pub unsafe extern "C" fn ttx_close_file(hooks: *mut tt::TTXFileHooks) {
    TTX_LITEX_STATE.with_borrow_mut(|s| {
        // SAFETY: Assumes TeraTerm passed us valid pointers.
        *(*hooks).PReadFile = s.orig_readfile;
        // *(*hooks).PWriteFile = s.orig_writefile;

        trace!(target: "TTXCloseFile", "*(*hooks).PReadFile <= {:?}", *(*hooks).PReadFile);
        // trace!(target: "TTXCloseFile", "*(*hooks).PWriteFile <= {:?}", *(*hooks).PWriteFile);
    });
}
