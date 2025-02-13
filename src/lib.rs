mod sfl;
mod teraterm;

use core::slice;
use std::ffi::{c_void, OsString};
use std::fmt::Write;
use std::fs::File;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::Mutex;
use std::{fmt, io, ptr};

use log::*;
use once_cell::sync::OnceCell;
use rfd::FileDialog;
use sfl::{MagicMatcher, SflLoader, MAGIC_RESPONSE};
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
pub const IDC_LITEX_ACTIVE: usize = 1005;

unsafe impl Send for State {}

struct State {
    ts: tt::PTTSet,
    cv: tt::PComVar,
    orig_readfile: tt::TReadFile,
    orig_writefile: tt::TWriteFile,
    file_menu: Option<HMENU>,
    transfer_menu: Option<HMENU>,
    activity: Activity,
    matcher: MagicMatcher,
    sfl_loader: Option<SflLoader<File>>,
    frames_sent: Option<u32>,
    frames_acked: Option<u32>,
    filename: Option<PathBuf>,
    addr: u32,
}

#[derive(PartialEq)]
enum Activity {
    Inactive,
    LookForMagic,
}

#[derive(Debug)]
enum Error {
    CouldntUnlock(&'static str),
    WasEmpty(&'static str),
    WinError(windows::core::Error),
    OutBuffOutOfBounds(u32),
    OutBuffFull { need: u32, actual: u32 },
    FileIoError(io::Error),
    BadAddressError(String),
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
            Error::OutBuffOutOfBounds(s) => {
                write!(
                    f,
                    "A write to TeraTerm's OutBuff would go out of bounds ({})",
                    s
                )
            }
            Error::OutBuffFull { need, actual } => {
                write!(
                    f,
                    "A write to TeraTerm's OutBuff would not fit (need {}, actual {})",
                    need, actual
                )
            }
            Error::FileIoError(e) => {
                write!(f, "Could not open or read kernel file: {}", e)
            }
            Error::BadAddressError(a) => {
                write!(
                    f,
                    "Could not intepret address as a decimal or hex integer: {}",
                    a
                )
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
                matcher: MagicMatcher::new(sfl::MAGIC),
                sfl_loader: None,
                frames_acked: None,
                frames_sent: None,
                filename: None,
                addr: 0x40000000,
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
            Activity::LookForMagic => {
                let chunk = slice::from_raw_parts(buff as *const u8, len as usize);
                if s.matcher.look_for_match(chunk) {
                    s.matcher.reset();
                    info!(target: "our_p_read_file", "Found magic string.");

                    match inject_output(s, &MAGIC_RESPONSE) {
                        Ok(()) => {}
                        Err(e) => error!("Could not inject magic response: {}", e),
                    }
                }
            }
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

fn set_dlg_osstring(dialog: HWND, control: i32) -> Result<OsString, windows::core::Error> {
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

            // Restore existing values.
            let (maybe_file, addr, active) = with_state_var(|s| {
                Ok((s.filename.clone(), s.addr, s.activity != Activity::Inactive))
            })
            .unwrap_or((None, 0x40000000, false));

            if let Some(file) = maybe_file {
                let mut file_vec: Vec<u16> = file.as_os_str().encode_wide().collect();
                file_vec.push(0);

                let _ = SetDlgItemTextW(dialog, IDC_LITEX_KERNEL as i32, PCWSTR(file_vec.as_ptr()));
            }

            let mut addr_str = String::new();
            let _ = write!(&mut addr_str, "{:#08x}", addr);
            let addr_os: OsString = addr_str.into();
            let mut addr_vec: Vec<u16> = addr_os.encode_wide().collect();
            addr_vec.push(0);

            let _ = SetDlgItemTextW(
                dialog,
                IDC_LITEX_BOOT_ADDR as i32,
                PCWSTR(addr_vec.as_ptr()),
            );

            let _ = SendDlgItemMessageW(
                dialog,
                IDC_LITEX_ACTIVE as i32,
                BM_SETCHECK,
                WPARAM(active.into()),
                LPARAM(0),
            );
            return true.into();
        }
        WM_COMMAND => match param_1.0 as i32 {
            p if p == IDOK.0 => {
                trace!(target: "setup_dialog", "OK");

                let active = SendDlgItemMessageW(
                    dialog,
                    IDC_LITEX_ACTIVE as i32,
                    BM_GETCHECK,
                    WPARAM(0),
                    LPARAM(0),
                )
                .0 != 0;

                let kernel_path = get_dlg_osstring(dialog, IDC_LITEX_KERNEL as i32)
                    .map(|kpath| PathBuf::from(kpath));

                let boot_addr = get_dlg_osstring(dialog, IDC_LITEX_BOOT_ADDR as i32)
                    .map_err(Error::WinError)
                    .and_then(|os| {
                        let boot_str = os.to_string_lossy().into_owned();

                        if boot_str.starts_with("0X") || boot_str.starts_with("0x") {
                            let no_prefix = &boot_str[2..];
                            u32::from_str_radix(no_prefix, 16)
                                .map_err(|_| Error::BadAddressError(boot_str))
                        } else {
                            if let Ok(addr) = u32::from_str_radix(&boot_str, 10) {
                                Ok(addr)
                            } else {
                                u32::from_str_radix(&boot_str, 16)
                                    .map_err(|_| Error::BadAddressError(boot_str))
                            }
                        }
                    });

                debug!(target: "setup_dialog", "Kernel Path: {:?}", kernel_path);
                debug!(target: "setup_dialog", "Boot Address: {:?}", boot_addr);
                debug!(target: "setup_dialog", "Active: {:?}", active);

                // FIXME: Error paths still need some tuning...
                match (kernel_path, boot_addr, active) {
                    (Ok(path), Ok(addr), true) => match SflLoader::open(path.clone(), addr) {
                        Ok(loader) => {
                            let _ = with_state_var(|s| {
                                s.filename = Some(path);
                                s.addr = addr;
                                s.sfl_loader = Some(loader);
                                s.matcher.reset();
                                s.activity = Activity::LookForMagic;
                                Ok(())
                            });

                            info!(target: "setup_dialog", "Plugin now actively searching for magic string.");
                        }
                        Err(e) => {
                            error!(target: "setup_dialog", "Could not open file: {}", e);
                        }
                    },
                    (kernel_path, addr, _) => {
                        if let Err(e) = kernel_path.as_ref() {
                            error!(target: "setup_dialog", "Bad filename: {}", e);

                            /* let mut err_str = String::new();
                            let _ = write!(&mut err_str, "Could not open file: {}", e);
                            error!(target: "setup_dialog", "{}", err_str);

                            let err_os: OsString = err_str.into();
                            let err_vec: Vec<u16> = err_os.encode_wide().collect();
                            MessageBoxW(Some(dialog), PCWSTR(err_vec.as_ptr()),
                             PCWSTR(u16cstr!("LiteX Setup").as_ptr()), MB_OK | MB_ICONWARNING | MB_APPLMODAL); */
                        }

                        let _ = with_state_var(|s| {
                            s.filename = kernel_path.ok();
                            s.addr = addr.unwrap_or(0x40000000);
                            s.activity = Activity::Inactive;
                            Ok(())
                        });
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
                if let Some(path) = FileDialog::new().pick_file() {
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
extern "C" fn TTXBind(_version: tt::WORD, exports: *mut tt::TTXExports) -> bool {
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
