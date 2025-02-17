/*! TeraTerm LiteX Serial Flash Loader Plugin. */

#[macro_use]
mod teraterm;  // Order matters: https://stackoverflow.com/a/29069165
mod error;
mod io;
mod sfl;
mod state;
mod ui;

use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::path::PathBuf;

use error::Error;
use log::*;
use parse_int::parse;
use sfl::{MagicMatcher, SflLoader};
use state::{Activity, State, OUR_HINST, TTX_LITEX_STATE};
use teraterm as tt;

use windows::Win32::Foundation::*;
use windows::Win32::System::SystemServices::*;

const TTX_EXPORTS: tt::TTXExports = tt::TTXExports {
    size: size_of::<tt::TTXExports>() as i32,
    loadOrder: 4600,
    TTXInit: Some(ttx_init),
    TTXGetUIHooks: None,
    TTXGetSetupHooks: None,
    TTXOpenTCP: None,
    TTXCloseTCP: None,
    TTXSetWinSize: None,
    TTXModifyMenu: Some(ui::ttx_modify_menu),
    TTXModifyPopupMenu: None,
    TTXProcessCommand: Some(ui::ttx_process_command),
    TTXEnd: None,
    TTXSetCommandLine: None,
    TTXOpenFile: Some(io::ttx_open_file),
    TTXCloseFile: Some(io::ttx_close_file),
};

#[no_mangle]
#[export_name = "TTXBind"]
unsafe extern "system" fn ttx_bind(_version: tt::WORD, exports: *mut tt::TTXExports) -> bool {
    // SAFETY: Assumes that TeraTerm gave us a proper struct.
    (&raw mut (*exports).loadOrder).write(TTX_EXPORTS.loadOrder);
    (&raw mut (*exports).TTXInit).write(TTX_EXPORTS.TTXInit);
    (&raw mut (*exports).TTXGetUIHooks).write(TTX_EXPORTS.TTXGetUIHooks);
    (&raw mut (*exports).TTXGetSetupHooks).write(TTX_EXPORTS.TTXGetSetupHooks);
    (&raw mut (*exports).TTXOpenTCP).write(TTX_EXPORTS.TTXOpenTCP);
    (&raw mut (*exports).TTXCloseTCP).write(TTX_EXPORTS.TTXCloseTCP);
    (&raw mut (*exports).TTXSetWinSize).write(TTX_EXPORTS.TTXSetWinSize);
    (&raw mut (*exports).TTXModifyMenu).write(TTX_EXPORTS.TTXModifyMenu);
    (&raw mut (*exports).TTXModifyPopupMenu).write(TTX_EXPORTS.TTXModifyPopupMenu);
    (&raw mut (*exports).TTXProcessCommand).write(TTX_EXPORTS.TTXProcessCommand);
    (&raw mut (*exports).TTXEnd).write(TTX_EXPORTS.TTXEnd);
    (&raw mut (*exports).TTXSetCommandLine).write(TTX_EXPORTS.TTXSetCommandLine);
    (&raw mut (*exports).TTXOpenFile).write(TTX_EXPORTS.TTXOpenFile);
    (&raw mut (*exports).TTXCloseFile).write(TTX_EXPORTS.TTXCloseFile);

    true
}

ttx_export! {
    unsafe fn ttx_init(ts: tt::PTTSet, cv: tt::PComVar) {
        if cfg!(debug_assertions) {
            let _ = stderrlog::new().verbosity(log::Level::Trace).init();
        } else {
            let _ = stderrlog::new().quiet(true).init();
        }

        let mut filename: Option<PathBuf> = None;
        let mut activity: Activity = Activity::Inactive;
        let mut addr = 0x40000000;
        let mut sfl_loader: Option<SflLoader<File>> = None;

        if cfg!(debug_assertions) {
            if let Ok(f) = env::var("TTX_LITEX_KERNEL") {
                debug!(target: "TTXInit", "Found TTX_LITEX_KERNEL override: {} {:?}", f, env::current_dir());
                addr = env::var("TTX_LITEX_ADDRESS")
                    .inspect_err(|e| error!(target: "TTXInit", "{}", e))
                    .ok()
                    .and_then(|s| {
                        parse::<u32>(&s)
                            .inspect_err(|e| error!(target: "TTXInit", "{}", e))
                            .ok()
                    })
                    .unwrap_or(addr);
                debug!(target: "TTXInit", "Address is {:#08X}", addr);

                let path = PathBuf::from(OsString::from(f));
                match SflLoader::open(path.clone(), addr) {
                    Ok(ldr) => {
                        debug!(target: "TTXInit", "Forcing TTXLiteX directly into LookForMagic state");
                        filename = Some(path);
                        activity = Activity::LookForMagic;
                        sfl_loader = Some(ldr);
                    }
                    Err(e) => {
                        error!(target: "TTXInit", "Could not force TTXLiteX into LookForMagic state: {}", e);
                    }
                }
            }
        }

        TTX_LITEX_STATE.set(State {
            ts,
            cv,
            orig_readfile: None,
            activity,
            matcher: MagicMatcher::new(sfl::MAGIC),
            sfl_loader,
            last_frame_acked: None,
            last_frame_sent: None,
            filename,
            addr,
            curr_frame: None,
            file_size: None
        });
    }
}

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(dll_module: HINSTANCE, call_reason: u32, _: *mut ()) -> bool {
    match call_reason {
        DLL_PROCESS_ATTACH => {
            let _ = OUR_HINST.set(dll_module);
        }
        DLL_PROCESS_DETACH => (),
        _ => (),
    }

    true
}
