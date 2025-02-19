/*! I/O hooks. */

use core::slice;
use std::ffi::c_void;
use std::fmt::Write;
use std::fs::File;
use std::time::Instant;
use std::{io, ptr};

use super::sfl::{Resp, SflLoader, MAGIC_RESPONSE};
use super::state::{Activity, State, TTX_LITEX_STATE};
use super::tt;
use super::Error;

use log::*;
use pretty_bytes_typed::pretty_bytes;
use windows::Win32::System::IO::OVERLAPPED;

enum ReadAction {
    PassThru,
    Swallow,
    Replace(String),
    Append(String),
    Prepend(String),
}

ttx_export! {
    #[allow(unused)]
    unsafe fn our_p_read_file(
        fh: *mut c_void,
        buff: *mut c_void,
        len: u32,
        read_bytes: *mut u32,
        wol: *mut OVERLAPPED,
    ) -> i32 {
        trace!(target: "our_p_read_file", "Entered");

        let mut rf_ret = 0;
        TTX_LITEX_STATE
            .with_borrow_mut(|mut s| {
                let read_file = s
                    .orig_readfile
                    .expect("PReadFile should've been set by TTXOpenFile");
                trace!(target: "our_p_read_file", "Running original PReadFile at {:?}.", read_file);
                rf_ret = read_file(fh, buff, len, read_bytes, wol);

                if *read_bytes == 0 {
                    return Ok(rf_ret);
                }

                // Not that InBuff is NOT circular either; ptr is "next value to be read",
                // and cnt is "num of values left to read". On entry, "CommReceive", which
                // is the immediate parent function of PReadFile, will move the unread part
                // of InBuff to offset 0 and set InPtr to 0 (See "Compact buffer" comment).
                // Trying to interfere with this by writing InBuff directly messes out
                // terminal output, so we inject anything we want to write to the
                // screen as the return value of our hook.
                let chunk = slice::from_raw_parts(buff as *const u8, *read_bytes as usize);
                match drive_sfl(&mut s, chunk)? {
                    ReadAction::PassThru => {}
                    ReadAction::Swallow => {
                        *read_bytes = 0;
                    }
                    ReadAction::Replace(s) => {
                        ptr::copy_nonoverlapping(s.as_ptr(), buff as *mut u8, s.len());
                        *read_bytes = s.len() as u32;
                    }
                    ReadAction::Append(s) => {
                        if (len - *read_bytes) >= (s.len() as u32) {
                            ptr::copy_nonoverlapping(
                                s.as_ptr(),
                                buff.offset(*read_bytes as isize) as *mut u8,
                                s.len(),
                            );
                            *read_bytes += s.len() as u32;
                        }
                    }
                    ReadAction::Prepend(s) => {
                        if (len - *read_bytes) >= (s.len() as u32) {
                            // XXX: Remove the last acknowledgment 'K' in the string. :)
                            // Maybe I should make an "ReplaceFirstAndPrepend" action.
                            *(buff as *mut u8) = b'\r';

                            ptr::copy(buff, buff.offset(s.len() as isize), *read_bytes as usize);
                            ptr::copy_nonoverlapping(s.as_ptr(), buff as *mut u8, s.len());
                            *read_bytes += s.len() as u32;
                        }
                    }
                }

                Ok::<_, Error>(rf_ret)
            })
            .inspect_err(|e| error!(target: "our_p_read_file", "Failed to drive SFL FSM: {}", e))
            .unwrap_or(rf_ret)
    }
}

fn drive_sfl(s: &mut State, chunk: &[u8]) -> Result<ReadAction, Error> {
    fn redo_last_frame(s: &mut State, err: Resp) -> Result<(), Error> {
        info!(target: "drive_sfl", "SFL Error: {}, resending current", err);
        let frame = s
            .curr_frame
            .take()
            .expect("a previous frame should've been saved before asking to redo a frame");
        trace!("resend: {:X?}", frame);
        inject_output(s, frame.as_bytes())?;
        s.curr_frame = Some(frame);

        Ok(())
    }

    fn status_bar(s: &State) -> String {
        let chunk_size = s
            .sfl_loader
            .as_ref()
            .expect("s.sfl_loader should have been initialized by Activity::LookForMagic")
            .chunk_size;
        let file_size = s
            .file_size
            .expect("s.file_size should have been initialized by Activity::LookForMagic");
        let mut total_chunks = file_size / (chunk_size as u64);
        let rem = file_size % (chunk_size as u64);
        if rem != 0 {
            total_chunks += 1;
        }

        const BAR_LENGTH: u64 = 40;
        let chunk_no = (s.last_frame_acked.unwrap() + 1) as u64;
        let used_part = (BAR_LENGTH * chunk_no) / total_chunks;

        let mut bar = String::with_capacity(BAR_LENGTH as usize);

        for _ in 0..used_part {
            bar.push('=');
        }

        // Arrow goes away once 100% loaded!
        if used_part != BAR_LENGTH {
            bar.push('>');
        }

        for _ in (used_part + 1)..BAR_LENGTH {
            bar.push(' ');
        }

        let mut resp = String::new();
        let _ = write!(
            &mut resp,
            "\r\x1B[0;36m[TTXLiteX] |{}| {} / {} chunks\x1B[0m",
            bar,
            s.last_frame_acked.unwrap() + 1,
            total_chunks
        );

        resp
    }

    match &mut s.activity {
        Activity::Inactive => Ok(ReadAction::PassThru),
        Activity::LookForMagic => {
            if !s.matcher.look_for_match(chunk) {
                return Ok(ReadAction::PassThru);
            }

            s.matcher.reset();
            info!(target: "drive_sfl", "Found magic string.");

            let filename = s.filename.as_ref().expect(
                "input filename should've been verified non-empty before Activity::LookForMagic",
            );

            let mut loader = File::open(filename)
                .map_err(|e| Error::FileIoError(e))
                .and_then(|fp| {
                    let size = fp.metadata().map_err(|e| Error::FileIoError(e))?.len();

                    if size == 0 {
                        return Err(Error::FileIoError(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "input file was empty",
                        )));
                    } else {
                        s.file_size = Some(size);
                        Ok(fp)
                    }
                })
                .map(|fp| SflLoader::new(fp, s.addr))?;

            inject_output(s, &MAGIC_RESPONSE)?;
            let frame = loader
                .encode_data_frame(0)
                .map_err(|e| Error::FileIoError(e))?
                .expect("input file should've been verified to be non-empty at this point");
            trace!("first: {:02X?}", frame);
            inject_output(s, frame.as_bytes())?;
            // Mutably borrowed twice???
            // fn next_data_frame(s: &mut State) -> Result<Option<Box<Frame>>, Error> {
            //     todo!()
            // }
            // inject_output(s, next_data_frame(s)?.unwrap().as_bytes())?;

            s.sfl_loader = Some(loader);
            s.activity = Activity::Calibrate;
            s.curr_frame = Some(frame);
            s.last_frame_sent = Some(0);
            s.start_time = Some(Instant::now());

            Ok(ReadAction::Append(
                "\r\x1B[0;36m[TTXLiteX] Uploading File\x1B[0m\r\n".to_string(),
            ))
        }
        Activity::Calibrate => {
            let loader = s
                .sfl_loader
                .as_mut()
                .expect("s.sfl_loader should have been initialized by Activity::LookForMagic");

            let action =
                match Resp::try_from(chunk[0]).map_err(|_| Error::UnexpectedResponse(chunk[0]))? {
                    Resp::Success => {
                        s.activity = Activity::WaitResp;
                        let mut resp = String::new();
                        let _ = write!(
                            resp,
                            "\x1B[0;36m[TTXLiteX] Using packet size: {} \x1B[0m\r\n",
                            loader.chunk_size
                        );
                        Ok(ReadAction::Replace(resp))
                    }
                    _ => {
                        info!(target: "drive_sfl", "Halved packet size.");
                        loader.halve_chunk_size();
                        Ok(ReadAction::Swallow)
                    }
                };

            // Resend frame 0 with final packet size to cleanly separate
            // calibration and send modes.
            let frame = loader
                .encode_data_frame(0)
                .map_err(|e| Error::FileIoError(e))?
                .expect("input file should've been verified to be non-empty at this point");

            inject_output(s, frame.as_bytes())?;
            action
        }
        Activity::WaitResp => {
            match Resp::try_from(chunk[0]).map_err(|_| Error::UnexpectedResponse(chunk[0]))? {
                Resp::Success => {
                    s.last_frame_acked = s.last_frame_sent;
                    let next_frame = s.last_frame_sent.expect(
                        "s.last_frame_sent should have been initialized by Activity::LookForMagic",
                    );

                    let loader = s.sfl_loader.as_mut().expect(
                        "s.sfl_loader should have been initialized by Activity::LookForMagic",
                    );

                    match loader
                        .encode_data_frame(next_frame + 1)
                        .map_err(|e| Error::FileIoError(e))?
                    {
                        Some(frame) => {
                            trace!("next: {:X?}", frame);
                            inject_output(s, frame.as_bytes())?;
                            s.curr_frame = Some(frame);
                            s.last_frame_sent = Some(next_frame + 1);
                        }
                        None => {
                            let frame = loader.encode_boot_frame(s.addr);
                            trace!("final: {:X?}", frame);
                            inject_output(s, frame.as_bytes())?;
                            s.curr_frame = Some(frame);
                            s.activity = Activity::WaitFinalResp;
                        }
                    };

                    let resp = status_bar(&s);
                    Ok(ReadAction::Replace(resp))
                }
                err @ (Resp::CrcError | Resp::Unknown | Resp::AckError) => {
                    redo_last_frame(s, err).map(|_| ReadAction::Swallow)
                }
            }
        }
        Activity::WaitFinalResp => {
            match Resp::try_from(chunk[0]).map_err(|_| Error::UnexpectedResponse(chunk[0]))? {
                Resp::Success => {
                    let file_size = s.file_size.expect(
                        "s.file_size should have been initialized by Activity::LookForMagic",
                    ) as f64;

                    s.file_size = None;
                    s.last_frame_acked = None;
                    s.last_frame_sent = None;
                    s.activity = Activity::LookForMagic;

                    let elapsed = (Instant::now() - s.start_time.unwrap()).as_secs_f64();
                    let rate = file_size / elapsed;

                    let mut resp = String::new();
                    let _ = write!(
                        resp,
                        "\r\n\x1B[0;36m[TTXLiteX] Done! ({}/s)\x1B[0m\r\n\r\n",
                        pretty_bytes(rate as u64, Some(2))
                    );

                    Ok(ReadAction::Prepend(resp))
                }
                err @ (Resp::CrcError | Resp::Unknown | Resp::AckError) => {
                    redo_last_frame(s, err).map(|_| ReadAction::Swallow)
                }
            }
        }
    }
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

ttx_export! {
    pub unsafe fn ttx_open_file(hooks: *mut tt::TTXFileHooks) {
        TTX_LITEX_STATE.with_borrow_mut(|s| {
            // SAFETY: Assumes TeraTerm passed us valid pointers.
            s.orig_readfile = *(*hooks).PReadFile;
            *(*hooks).PReadFile = Some(our_p_read_file);

            trace!(target: "TTXOpenFile", "s.orig_readfile <= {:?} ({:?})", &raw const s.orig_readfile, s.orig_readfile);
            trace!(target: "TTXOpenFile", "*(*hooks).PReadFile <= {:?}", our_p_read_file as * const ());
        });
    }
}

ttx_export! {
    pub unsafe fn ttx_close_file(hooks: *mut tt::TTXFileHooks) {
        TTX_LITEX_STATE.with_borrow_mut(|s| {
            // SAFETY: Assumes TeraTerm passed us valid pointers, and that
            // TeraTerm calls this function _after_ TTXOpenFile.
            *(*hooks).PReadFile = s.orig_readfile;

            trace!(target: "TTXCloseFile", "*(*hooks).PReadFile <= {:?}", *(*hooks).PReadFile);
        });
    }
}
