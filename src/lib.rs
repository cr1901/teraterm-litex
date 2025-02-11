mod teraterm;

use std::ffi::CStr;
use std::path::PathBuf;
use std::sync::Mutex;

use once_cell::sync::OnceCell;
use rfd::FileDialog;
use teraterm as tt;

use windows::core::PCSTR;
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
    match TTX_LITEX_STATE.try_lock() {
        Ok(mut g) => {
            match &mut *g {
                Some(ref mut s) => {
                    s.file_menu = Some(GetSubMenu(menu, tt::ID_FILE as i32));
                    // ID_TRANSFER == 9 and doesn't work. Was the constant
                    // never updated?
                    s.transfer_menu = Some(GetSubMenu(s.file_menu.unwrap(), 11));

                    let res = AppendMenuA(
                        s.transfer_menu.unwrap(),
                        MF_ENABLED | MF_STRING,
                        ID_MENU_LITEX,
                        PCSTR(c"LiteX".as_ptr() as *const u8),
                    );
                    if let Err(r) = res {
                        eprintln!("Could not add menu: {}", r);
                    }
                }
                None => {
                    eprintln!("Could not unlock state. Plugin cannot do anything.");
                }
            }
        }
        Err(_) => {
            eprintln!("Could not modify menu. Plugin cannot do anything.");
        }
    }
}

unsafe extern "system" fn litex_setup_dialog(
    window: HWND,
    msg: u32,
    param_1: WPARAM,
    _param_2: LPARAM,
) -> isize {
    match msg {
        WM_INITDIALOG => {
            // TODO:
            // * Init with already-filled values.
            // * Center Window

            // SendMessage(EM_SETLIMITTEXT);
            return true.into();
        }
        WM_COMMAND => {
            match param_1.0 as i32 {
                p if p == IDOK.0 => {
                    let mut kernel_file: [u8; 256] = [0; 256];
                    let _ =
                        GetDlgItemTextA(window, IDC_LITEX_KERNEL as i32, &mut kernel_file[0..255]);

                    let mut rust_str = String::from_utf8_lossy(&kernel_file).into_owned();

                    if let Some((first, _)) = rust_str.char_indices().find(|(_, c)| *c == '\0') {
                        rust_str.truncate(first);
                    }

                    let kernel_path = Some(PathBuf::from(rust_str));

                    eprintln!("Got kernel path: {:?}", kernel_path);

                    let mut boot_bytes: [u8; 17] = [0; 17];
                    let mut boot_addr = None;
                    let _ =
                        GetDlgItemTextA(window, IDC_LITEX_BOOT_ADDR as i32, &mut boot_bytes[0..16]);

                    // Try a few ways of parsing the boot address.
                    if let Ok(boot_cstr) = CStr::from_bytes_until_nul(&boot_bytes) {
                        if let Ok(boot_str) = boot_cstr.to_str() {
                            if boot_str.starts_with("0X") || boot_str.starts_with("0x") {
                                let no_prefix = &boot_str[2..];

                                if let Ok(addr) = u32::from_str_radix(no_prefix, 16) {
                                    boot_addr = Some(addr);
                                }
                            } else {
                                if let Ok(addr) = u32::from_str_radix(boot_str, 10) {
                                    boot_addr = Some(addr);
                                } else if let Ok(addr) = u32::from_str_radix(boot_str, 16) {
                                    boot_addr = Some(addr);
                                }
                            }

                            eprintln!("Got boot address: {:?}", boot_addr)
                        }
                    }

                    if kernel_path.is_some() && boot_addr.is_some() {
                        match TTX_LITEX_STATE.try_lock() {
                            Ok(mut g) => match &mut *g {
                                Some(ref mut s) => {
                                    s.activity = Activity::Active {
                                        file: kernel_path.unwrap(),
                                        boot_addr: boot_addr.unwrap(),
                                    };
                                }
                                None => {
                                    eprintln!("Could not unlock state. Plugin cannot do anything.");
                                }
                            },
                            Err(_) => {
                                eprintln!("Could not modify menu. Plugin cannot do anything.");
                            }
                        }
                    }

                    let _ = EndDialog(window, IDOK.0 as isize);
                    return true.into();
                }
                p if p == IDCANCEL.0 => {
                    let _ = EndDialog(window, IDCANCEL.0 as isize);
                    return true.into();
                }
                p if p == IDC_LITEX_CHOOSE_KERNEL_BUTTON as i32 => {
                    if let Some(path) = FileDialog::new().add_filter("kernel", &["bin"]).pick_file()
                    {
                        if let Err(e) = SetDlgItemTextA(
                            window,
                            IDC_LITEX_KERNEL as i32,
                            PCSTR(path.to_string_lossy().as_bytes().as_ptr()),
                        ) {
                            eprintln!("Could not set kernel file path: {}", e);
                        }
                    }
                    eprintln!("Choose Kernel Button");
                }
                _ => {}
            }
        }
        _ => {}
    }

    return false.into();
}

unsafe extern "C" fn ttx_process_command(window: HWND, cmd: u16) -> i32 {
    match cmd as usize {
        ID_MENU_LITEX => {
            if let Ok(hc) = OUR_HINST.try_lock() {
                if let Some(hinst) = hc.get() {
                    eprintln!("Invoking dialog box.");
                    let res = DialogBoxParamA(
                        Some(hinst.0),
                        PCSTR(IDD_SETUP_LITEX as *const u8),
                        Some(window),
                        Some(Some(litex_setup_dialog)),
                        LPARAM(0),
                    );
                    if res <= 0 {
                        eprintln!("error invoking dialog: {:?}", GetLastError());
                    }
                } else {
                    eprintln!("Could not get OurHInstance.");
                }
            } else {
                eprintln!("Could not unlock OurHInstance.");
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
