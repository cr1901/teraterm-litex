mod sfl;
mod teraterm;

use core::slice;
use std::ffi::{c_void, OsString};
use std::fmt;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Mutex;

use log::*;
use once_cell::sync::OnceCell;
use rfd::FileDialog;
use sfl::MagicMatcher;
use stderrlog;
use teraterm as tt;

use widestring::{u16cstr, U16CString};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::SystemServices::*;
use windows::Win32::System::IO::OVERLAPPED;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ID_MENU_LITEX: usize = 56000;
pub const IDD_SETUP_LITEX: usize = 1001;
pub const IDC_LITEX_KERNEL: usize = 1002;
pub const IDC_LITEX_BOOT_ADDR: usize = 1003;
pub const IDC_LITEX_CHOOSE_KERNEL_BUTTON: usize = 1004;

unsafe impl Send for State {}

struct State {
    ts: tt::PTTSet,
    cv: tt::PComVar,
    orig_readfile: tt::TReadFile,
    orig_writefile: tt::TWriteFile,
    file_menu: Option<HMENU>,
    transfer_menu: Option<HMENU>,
    activity: Activity,
}

enum Activity {
    Inactive,
    Active { file: PathBuf, boot_addr: u32, matcher: MagicMatcher },
}

enum Error {
    CouldntUnlock(&'static str),
    WasEmpty(&'static str),
    WinError(windows::core::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::CouldntUnlock(var) => {
                write!(f, "couldnt unlock state var {}", var)
            }
            Error::WasEmpty(var) => {
                write!(f, "state var {} was empty when it shouldn't have", var)
            }
            Error::WinError(e) => {
                write!(f, "Windows error: {}", e)
            }
        }
    }
}

unsafe impl Send for OurHInstance {}

struct OurHInstance(HINSTANCE);

static TTX_LITEX_STATE: Mutex<Option<State>> = Mutex::new(None);
static OUR_HINST: Mutex<OnceCell<OurHInstance>> = Mutex::new(OnceCell::new());

unsafe extern "C" fn ttx_init(ts: tt::PTTSet, cv: tt::PComVar) {
    if cfg!(debug_assertions) {
        let _ = stderrlog::new().verbosity(log::Level::Trace).init();
    } else {
        let _ = stderrlog::new().quiet(true).init();
    }

    match TTX_LITEX_STATE.try_lock() {
        Ok(mut s) => {
            *s = Some(State {
                ts,
                cv,
                orig_readfile: None,
                orig_writefile: None,
                file_menu: None,
                transfer_menu: None,
                activity: Activity::Inactive,
            })
        }
        Err(_) => {
            error!(target: "TTXInit", "Could not set state. Plugin cannot do anything.");
        }
    }
}

unsafe extern "C" fn ttx_open_file(hooks: *mut tt::TTXFileHooks) {
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

unsafe extern "C" fn ttx_close_file(hooks: *mut tt::TTXFileHooks) {
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
            Activity::Active { file, boot_addr , matcher} => {
                let chunk = slice::from_raw_parts(buff as *const u8, len as usize);
                if matcher.look_for_match(chunk) {
                    info!(target: "our_p_read_file", "Found magic string.");

                }
            },
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

unsafe extern "C" fn ttx_modify_menu(menu: HMENU) {
    if let Err(e) = with_state_var(|s| {
        s.file_menu = Some(GetSubMenu(menu, tt::ID_FILE as i32));
        // ID_TRANSFER == 9 and doesn't work. Was the constant
        // never updated?
        s.transfer_menu = Some(GetSubMenu(s.file_menu.unwrap(), 11));

        AppendMenuW(
            s.transfer_menu.unwrap(),
            MF_ENABLED | MF_STRING,
            ID_MENU_LITEX,
            PCWSTR(u16cstr!("LiteX").as_ptr()),
        )
        .map_err(|e| Error::WinError(e))?;

        Ok(())
    }) {
        debug!(target: "TTXModifyMenu", "Could not modify menu: {}", e);
    }
}

fn get_buf_len(dialog: HWND, control: i32) -> Result<usize, windows::core::Error> {
    let control_handle = unsafe { GetDlgItem(Some(dialog), control)? };
    let maybe_buf_len = unsafe { GetWindowTextLengthW(control_handle) };

    let maybe_error = windows::core::Error::from_win32();
    if maybe_buf_len == 0 && maybe_error.code().0 != 0 {
        return Err(maybe_error);
    } else {
        return Ok(maybe_buf_len as usize);
    }
}

fn get_dlg_osstring(dialog: HWND, control: i32) -> Result<OsString, windows::core::Error> {
    let code_unit_len = get_buf_len(dialog, control)?;
    let mut code_str: Vec<u16> = vec![0; code_unit_len + 1];
    let used_len = unsafe { GetDlgItemTextW(dialog, control, &mut code_str) } as usize;

    // We don't need the null terminator.
    code_str.truncate(used_len);

    if used_len == 0 {
        return Err(windows::core::Error::from_win32());
    } else {
        return Ok(OsString::from_wide(&code_str));
    }
}

unsafe extern "system" fn litex_setup_dialog(
    dialog: HWND,
    msg: u32,
    param_1: WPARAM,
    _param_2: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            // TODO:
            // * Center Window
            // * SendMessage(EM_SETLIMITTEXT);
            let _ = SetDlgItemTextW(
                dialog,
                IDC_LITEX_BOOT_ADDR as i32,
                PCWSTR(u16cstr!("0x40000000").as_ptr()),
            );
            return true.into();
        }
        WM_COMMAND => match param_1.0 as i32 {
            p if p == IDOK.0 => {
                trace!(target: "setup_dialog", "OK");

                let kernel_path = match get_dlg_osstring(dialog, IDC_LITEX_KERNEL as i32) {
                    Ok(kpath) => Some(PathBuf::from(kpath)),
                    Err(e) => {
                        error!(target: "setup_dialog", "Could not get kernel path buffer {:?}", e);
                        None
                    }
                };

                let boot_addr = match get_dlg_osstring(dialog, IDC_LITEX_BOOT_ADDR as i32)
                    .map(|os| os.to_string_lossy().into_owned())
                {
                    Ok(boot_str) => {
                        // Attempt to parse various forms of the address.
                        if boot_str.starts_with("0X") || boot_str.starts_with("0x") {
                            let no_prefix = &boot_str[2..];
                            u32::from_str_radix(no_prefix, 16).ok()
                        } else {
                            if let Ok(addr) = u32::from_str_radix(&boot_str, 10) {
                                Some(addr)
                            } else {
                                u32::from_str_radix(&boot_str, 16).ok()
                            }
                        }
                    }
                    Err(e) => {
                        error!(target: "setup_dialog", "Could not get boot address: {:?}", e);
                        None
                    }
                };

                debug!(target: "setup_dialog", "Got kernel path: {:?}", kernel_path);
                debug!(target: "setup_dialog", "Got boot address: {:?}", boot_addr);

                if kernel_path.is_some() && boot_addr.is_some() {
                    if let Err(e) = with_state_var(|s| {
                        info!(target: "setup_dialog", "Plugin now actively searching for magic string.");
                        s.activity = Activity::Active {
                            file: kernel_path.unwrap(),
                            boot_addr: boot_addr.unwrap(),
                            matcher: MagicMatcher::new(sfl::MAGIC)
                        };
                        Ok(())
                    }) {
                        error!(target: "setup_dialog", "Could not move plugin to active state: {}", e);
                    }
                }

                let _ = EndDialog(dialog, IDOK.0 as isize);
                return true.into();
            }
            p if p == IDCANCEL.0 => {
                trace!(target: "setup_dialog", "Cancel");
                let _ = EndDialog(dialog, IDCANCEL.0 as isize);
                return true.into();
            }
            p if p == IDC_LITEX_CHOOSE_KERNEL_BUTTON as i32 => {
                trace!(target: "setup_dialog", "Choose Kernel");
                if let Some(path) = FileDialog::new().add_filter("kernel", &["bin"]).pick_file() {
                    let widepath = U16CString::from_os_str_truncate(path.as_os_str());
                    if let Err(e) =
                        SetDlgItemTextW(dialog, IDC_LITEX_KERNEL as i32, PCWSTR(widepath.as_ptr()))
                    {
                        error!(target: "setup_dialog", "Could not set kernel file path: {}", e);
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }

    return false.into();
}

fn with_state_var<T, F>(f: F) -> Result<T, Error>
where
    F: FnOnce(&mut State) -> Result<T, Error>,
{
    let mut state_guard = TTX_LITEX_STATE
        .try_lock()
        .map_err(|_| Error::CouldntUnlock("TTXState"))?;

    let state = state_guard.as_mut().ok_or(Error::WasEmpty("TTXState"))?;

    f(state)
}

fn get_hinst_var() -> Result<HINSTANCE, Error> {
    let hinst_guard = OUR_HINST
        .try_lock()
        .map_err(|_| Error::CouldntUnlock("OurHInstance"))?;
    let hinst = hinst_guard.get().ok_or(Error::WasEmpty("OurHInstance"))?;

    Ok(hinst.0)
}

unsafe extern "C" fn ttx_process_command(window: HWND, cmd: u16) -> i32 {
    match cmd as usize {
        ID_MENU_LITEX => {
            debug!(target: "TTXProcessCommand", "LiteX option clicked.");

            if let Err(e) = get_hinst_var().and_then(|hinst| {
                let res = DialogBoxParamW(
                    Some(hinst),
                    PCWSTR(IDD_SETUP_LITEX as *const u16),
                    Some(window),
                    Some(Some(litex_setup_dialog)),
                    LPARAM(0),
                );

                if res <= 0 {
                    Err(Error::WinError(GetLastError().into()))
                } else {
                    Ok(())
                }
            }) {
                debug!(target: "TTXProcessCommand", "Could not open LiteX dialog: {}", e)
            }

            return 1;
        }
        _ => {
            return 0;
        }
    }
}

const TTX_EXPORTS: tt::TTXExports = tt::TTXExports {
    size: size_of::<tt::TTXExports>() as i32,
    loadOrder: 4600,
    TTXInit: Some(ttx_init),
    TTXGetUIHooks: None,
    TTXGetSetupHooks: None,
    TTXOpenTCP: None,
    TTXCloseTCP: None,
    TTXSetWinSize: None,
    TTXModifyMenu: Some(ttx_modify_menu),
    TTXModifyPopupMenu: None,
    TTXProcessCommand: Some(ttx_process_command),
    TTXEnd: None,
    TTXSetCommandLine: None,
    TTXOpenFile: Some(ttx_open_file),
    TTXCloseFile: Some(ttx_close_file),
};

#[no_mangle]
#[allow(non_snake_case)]
fn TTXBind(_version: tt::WORD, exports: *mut tt::TTXExports) -> bool {
    // SAFETY: Assumes that TeraTerm gave us a proper struct.
    unsafe {
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
    }

    true
}

#[no_mangle]
#[allow(non_snake_case, unused_variables)]
extern "system" fn DllMain(dll_module: HINSTANCE, call_reason: u32, _: *mut ()) -> bool {
    match call_reason {
        DLL_PROCESS_ATTACH => {
            if let Ok(hinst) = OUR_HINST.try_lock() {
                let _ = hinst.set(OurHInstance(dll_module));
            }
        }
        DLL_PROCESS_DETACH => (),
        _ => (),
    }

    true
}
