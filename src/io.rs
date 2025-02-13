/*! I/O hooks. */

use core::slice;
use std::ffi::c_void;
use std::ptr;

use log::*;
use super::sfl::MAGIC_RESPONSE;
use super::state::{with_state_var, Activity, State};
use super::tt;
use super::Error;

use windows::Win32::System::IO::OVERLAPPED;
use zerocopy::IntoBytes;

#[allow(unused)]
unsafe extern "C" fn our_p_write_file(
    fh: *mut c_void,
    buff: *const c_void,
    len: u32,
    written_bytes: *mut u32,
    wol: *mut OVERLAPPED,
) -> i32 {
    trace!(target: "our_p_write_file", "Entered");

    match with_state_var(|s| {
        match (&mut s.activity, s.last_frame_sent, s.last_frame_acked, s.sfl_loader.as_mut()) {
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
            },
            (Activity::WriteAndWait, Some(sent), Some(acked), Some(loader)) if sent == acked => {
                if let Err(e) = loader
                    .encode_data_frame(sent)
                    .map_err(|e| Error::FileIoError(e))
                    .and_then(|(len, frame)| {
                        inject_output(s, &frame.as_bytes()[..len])?;
                        s.last_frame_sent = Some(sent + 1);

                        Ok(())
                    }) {
                        error!("Could not send packet {}: {}", sent, e);
                    }
            },
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
}

#[allow(unused)]
unsafe extern "C" fn our_p_read_file(
    fh: *mut c_void,
    buff: *mut c_void,
    len: u32,
    read_bytes: *mut u32,
    wol: *mut OVERLAPPED,
) -> i32 {
    trace!(target: "our_p_read_file", "Entered");

    match with_state_var(|s| {
        match &mut s.activity {
            Activity::LookForMagic => {
                let chunk = slice::from_raw_parts(buff as *const u8, len as usize);
                if s.matcher.look_for_match(chunk) {
                    s.matcher.reset();
                    info!(target: "our_p_read_file", "Found magic string.");

                    match inject_output(s, &MAGIC_RESPONSE) {
                        Ok(()) => {
                            s.activity = Activity::WriteAndWait;
                        }
                        Err(e) => error!("Could not inject magic response: {}", e),
                    }
                }
            }
            Activity::WriteAndWait => {}
            _ => {}
        }

        if let Some(read_file) = s.orig_readfile {
            return Ok(read_file);
        } else {
            return Err(Error::WasEmpty("PReadFile"));
        }
    }) {
        Ok(rf) => {
            trace!(target: "our_p_read_file", "Running original PReadFile at {:?}.", rf);
            return rf(fh, buff, len, read_bytes, wol);
        }
        Err(e) => {
            error!(target: "our_p_read_file", "Could not call original PWriteFile: {}", e);
            return 0;
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


pub unsafe extern "C" fn ttx_open_file(hooks: *mut tt::TTXFileHooks) {
    if let Err(e) = with_state_var(|s| {
        // SAFETY: Assumes TeraTerm passed us valid pointers.
        s.orig_readfile = *(*hooks).PReadFile;
        s.orig_writefile = *(*hooks).PWriteFile;
        *(*hooks).PReadFile = Some(our_p_read_file);
        *(*hooks).PWriteFile = Some(our_p_write_file);

        trace!(target: "TTXOpenFile", "s.orig_readfile <= {:?} ({:?})", &raw const s.orig_readfile, s.orig_readfile);
        trace!(target: "TTXOpenFile", "s.orig_writefile <= {:?} ({:?})", &raw const s.orig_writefile, s.orig_writefile);
        trace!(target: "TTXOpenFile", "*(*hooks).PReadFile <= {:?}", our_p_read_file as * const ());
        trace!(target: "TTXOpenFile", "*(*hooks).PWriteFile <= {:?}", our_p_write_file as * const ());

        Ok(())
    }) {
        error!(target: "TTXOpenFile", "Could not prepare serial port: {}", e);
    }
}

pub unsafe extern "C" fn ttx_close_file(hooks: *mut tt::TTXFileHooks) {
    if let Err(e) = with_state_var(|s| {
        // SAFETY: Assumes TeraTerm passed us valid pointers.
        *(*hooks).PReadFile = s.orig_readfile;
        *(*hooks).PWriteFile = s.orig_writefile;

        trace!(target: "TTXCloseFile", "*(*hooks).PReadFile <= {:?}", *(*hooks).PReadFile);
        trace!(target: "TTXCloseFile", "*(*hooks).PWriteFile <= {:?}", *(*hooks).PWriteFile);

        Ok(())
    }) {
        error!(target: "TTXCloseFile", "Could not close serial port: {}", e);
    }
}

