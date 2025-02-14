/*! I/O hooks. */

use core::slice;
use std::ffi::c_void;
use std::{io, ptr};

use super::sfl::{Resp, MAGIC_RESPONSE};
use super::state::{TTX_LITEX_STATE, Activity, State};
use super::tt;
use super::Error;
use log::*;

use windows::Win32::System::IO::OVERLAPPED;
use zerocopy::IntoBytes;

/* #[allow(unused)]
unsafe extern "C" fn our_p_write_file(
    fh: *mut c_void,
    buff: *const c_void,
    len: u32,
    written_bytes: *mut u32,
    wol: *mut OVERLAPPED,
) -> i32 {
    trace!(target: "our_p_write_file", "Entered");

    match with_state_var(|s| {
        match (
            &mut s.activity,
            s.last_frame_sent,
            s.last_frame_acked,
            s.sfl_loader.as_mut(),
        ) {
            (Activity::WriteAndWait, None, None, Some(loader)) => {
                let buf = slice::from_raw_parts(buff as *const u8, len as usize);

                if let Err(e) = loader
                    .encode_data_frame(0)
                    .map_err(|e| Error::FileIoError(e))
                    .and_then(|(len, frame)| {
                        trace!(target: "our_p_write_file", "Injecting packet: {:#X?}", &frame.as_bytes()[..len]);
                        inject_output(s, &frame.as_bytes()[..len])?;
                        s.last_frame_sent = Some(0);

                        Ok(())
                    }) {
                        error!("Could not send packet 0: {}", e);
                    }
            }
            (Activity::WriteAndWait, Some(sent), Some(acked), Some(loader)) if sent == acked => {
                if let Err(e) = loader
                    .encode_data_frame(sent)
                    .map_err(|e| Error::FileIoError(e))
                    .and_then(|(len, frame)| {
                        inject_output(s, &frame.as_bytes()[..len])?;
                        s.last_frame_sent = Some(sent + 1);

                        Ok(())
                    })
                {
                    error!("Could not send packet {}: {}", sent, e);
                }
            }
            _ => {}
        }

        if let Some(write_file) = s.orig_writefile {
            return Ok(write_file);
        } else {
            return Err(Error::WasEmpty("PWriteFile"));
        }
    }) {
        Ok(wf) => {
            trace!(target: "our_p_write_file", "Running original PWriteFile at {:?}.", wf);
            return wf(fh, buff, len, written_bytes, wol);
        }
        Err(e) => {
            error!(target: "our_p_write_file", "Could not call original PWriteFile: {}", e);
            return 0;
        }
    }
} */

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
    TTX_LITEX_STATE.with_borrow_mut(|mut s| {
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
        Activity::Inactive => {},
        Activity::LookForMagic => {
            if s.matcher.look_for_match(chunk) {
                s.matcher.reset();
                info!(target: "drive_sfl", "Found magic string.");

                return inject_output(s, &MAGIC_RESPONSE)
                    // .inspect_err(|e| error!("Could not inject magic response: {}", e))
                    .and_then(|_| {
                        let (used, frame) = s
                            .sfl_loader
                            .as_mut()
                            .ok_or(Error::WasEmpty("loader"))?
                            .encode_data_frame(0)
                            .map_err(|e| Error::FileIoError(e))?
                            .ok_or(Error::FileIoError(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "input file was empty",
                            )))?;
                        trace!("{:X?}", frame);
                        inject_output(s, &frame.as_bytes()[..used])?;

                        s.last_frame_sent = Some(0);
                        s.activity = Activity::WaitResp;

                        Ok(())
                    });
                // .inspect_err(|e| error!("Could not write initial packet: {}", e));
            } else {
                return Ok(());
            }
        }
        Activity::WaitResp => {
            match Resp::try_from(chunk[0]).map_err(|_| Error::UnexpectedResponse(chunk[0]))? {
                Resp::Success => {
                    s.last_frame_acked = Some(s.last_frame_acked.map_or(0, |ack| ack + 1));

                    let loader = s.sfl_loader.as_mut().ok_or(Error::WasEmpty("loader"))?;

                    let next_frame = s.last_frame_sent.map_or(0, |sent| sent + 1);
                    if let Some((used, frame)) = loader.encode_data_frame(next_frame).map_err(|e| Error::FileIoError(e))? {
                        trace!("{:X?}", frame);
                        inject_output(s, &frame.as_bytes()[..used])?;
                        s.last_frame_sent = Some(next_frame);
                    } else {
                        let (used, frame) = loader.encode_boot_frame(s.addr);
                        inject_output(s, &frame.as_bytes()[..used])?;
                        s.activity = Activity::WaitFinalResp;
                    }
                    // if let Some((used, frame)) =
                    //     .encode_data_frame(0)
                    //     .map_err(|e| Error::FileIoError(e))?
                    // {
                    //     inject_output(s, &frame.as_bytes()[..used])?;
                    //     s.last_frame_sent = Some(s.last_frame_sent.map_or(0, |sent| sent + 1));
                    // } else {

                    // }
                }
                Resp::CrcError | Resp::Unknown | Resp::AckError => {
                    // frame_no = Some(s.last_frame_acked.map_or(0, |ack| ack + 1));
                    // info!(target: "drive_sfl", "CRC Error.");
                    // let (used, frame) = s
                    //         .sfl_loader
                    //         .as_mut()
                    //         .ok_or(Error::WasEmpty("loader"))?
                    //         .encode_data_frame(0)
                    //         .map_err(|e| Error::FileIoError(e))?
                    //         .ok_or(Error::FileIoError(io::Error::new(
                    //             io::ErrorKind::UnexpectedEof,
                    //             "input file was empty",
                    //         )))?;
                    // inject_output(s, &frame.as_bytes()[..used])?;
                }
            }
            // .inspect_err(|e| error!("Could not inject magic response: {}", e));

            /* Only check the initial frame so we know whether we  */
        },
        Activity::WaitFinalResp => {
            match Resp::try_from(chunk[0]).map_err(|_| Error::UnexpectedResponse(chunk[0]))? {
                Resp::Success => {
                    s.last_frame_acked = None;
                    s.last_frame_sent = None;
                    s.activity = Activity::LookForMagic;
                }
                Resp::CrcError | Resp::Unknown | Resp::AckError => {

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
