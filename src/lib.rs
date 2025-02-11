mod teraterm;

use std::ffi::OsString;
use std::fmt;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::sync::Mutex;

use once_cell::sync::OnceCell;
use rfd::FileDialog;
use teraterm as tt;

use widestring::{u16cstr, U16CString};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::SystemServices::*;
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
    Active { file: PathBuf, boot_addr: u32 },
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
            eprintln!("Could not set state. Plugin cannot do anything.");
        }
    }
}

unsafe extern "C" fn ttx_openfile(hooks: *mut tt::TTXFileHooks) {
    // SAFETY assumes that TeraTerm gave us a proper struct.

    todo!()
    // (&raw mut (*hooks).PReadFile).write(TTX_EXPORTS.loadOrder);
    // (&raw mut (*hooks).PWriteFile).write(TTX_EXPORTS.loadOrder);
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
        ).map_err(|e| Error::WinError(e))?;

        Ok(())
    }) {
        eprintln!("Could not modify menu: {}", e);
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
                let kernel_path = match get_dlg_osstring(dialog, IDC_LITEX_KERNEL as i32) {
                    Ok(kpath) => Some(PathBuf::from(kpath)),
                    Err(e) => {
                        eprintln!("Could not get kernel path buffer {:?}", e);
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
                        eprintln!("Could not get boot address: {:?}", e);
                        None
                    }
                };

                eprintln!("Got kernel path: {:?}", kernel_path);
                eprintln!("Got boot address: {:?}", boot_addr);

                if kernel_path.is_some() && boot_addr.is_some() {
                    if let Err(e) = with_state_var(|s| {
                        eprintln!("Plugin now actively searching for magic string.");
                        s.activity = Activity::Active {
                            file: kernel_path.unwrap(),
                            boot_addr: boot_addr.unwrap()
                        };
                        Ok(())
                    }) {
                        eprintln!("Could not move plugin to active state: {}", e);
                    }
                }

                let _ = EndDialog(dialog, IDOK.0 as isize);
                return true.into();
            }
            p if p == IDCANCEL.0 => {
                let _ = EndDialog(dialog, IDCANCEL.0 as isize);
                return true.into();
            }
            p if p == IDC_LITEX_CHOOSE_KERNEL_BUTTON as i32 => {
                if let Some(path) = FileDialog::new().add_filter("kernel", &["bin"]).pick_file() {
                    let widepath = U16CString::from_os_str_truncate(path.as_os_str());
                    if let Err(e) =
                        SetDlgItemTextW(dialog, IDC_LITEX_KERNEL as i32, PCWSTR(widepath.as_ptr()))
                    {
                        eprintln!("Could not set kernel file path: {}", e);
                    }
                }
                eprintln!("Choose Kernel Button");
            }
            _ => {}
        },
        _ => {}
    }

    return false.into();
}

fn with_state_var<F>(f: F) -> Result<(), Error> where F: FnOnce(&mut State) -> Result<(), Error> {
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
                eprintln!("Could not open LiteX dialog: {}", e)
            }

            eprintln!("LiteX option clicked.");
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
    TTXOpenFile: None,
    TTXCloseFile: None,
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
