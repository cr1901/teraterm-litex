mod teraterm;

use std::sync::Mutex;

use once_cell::sync::OnceCell;
use teraterm as tt;

use windows::core::PCSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::SystemServices::*;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuA, DialogBoxParamA, GetSubMenu, HMENU, MF_ENABLED, MF_STRING,
};

pub const ID_MENU_LITEX: usize = 56000;
pub const IDD_SETUP_LITEX: usize = 1001;

unsafe impl Send for State {}

struct State {
    ts: tt::PTTSet,
    cv: tt::PComVar,
    orig_readfile: tt::TReadFile,
    orig_writefile: tt::TWriteFile,
    file_menu: Option<HMENU>,
    transfer_menu: Option<HMENU>,
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

                    let litex_str = c"LiteX".as_ptr() as *const u8;
                    let res = AppendMenuA(
                        s.transfer_menu.unwrap(),
                        MF_ENABLED | MF_STRING,
                        ID_MENU_LITEX,
                        PCSTR(litex_str),
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
                        None,
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
