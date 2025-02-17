/*! UI and Window-creation hooks. */

use std::ffi::OsString;
use std::fmt::Write;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use log::*;
use parse_int::parse;
use rfd::FileDialog;

use super::state::{Activity, OUR_HINST, TTX_LITEX_STATE};
use super::tt;
use super::Error;

use widestring::{u16cstr, U16CString};
use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const ID_MENU_LITEX: usize = 56000;
pub const IDD_SETUP_LITEX: usize = 1001;
pub const IDC_LITEX_KERNEL: usize = 1002;
pub const IDC_LITEX_BOOT_ADDR: usize = 1003;
pub const IDC_LITEX_CHOOSE_KERNEL_BUTTON: usize = 1004;
pub const IDC_LITEX_ACTIVE: usize = 1005;

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

    if used_len == 0 && windows::core::Error::from_win32().code().0 != 0 {
        return Err(windows::core::Error::from_win32());
    } else {
        return Ok(OsString::from_wide(&code_str));
    }
}

pub unsafe extern "system" fn litex_setup_dialog(
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
            let (maybe_file, addr, active) = TTX_LITEX_STATE
                .with_borrow(|s| (s.filename.clone(), s.addr, s.activity != Activity::Inactive));

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

                // TODO: If both are clear, Windows returns "Handle is invalid" for both.
                // If only path is clear, Windows returns "Handle is invalid" for path.
                // If only address is clear, Windows returns empty string for address.
                // Why?
                let kernel_path = get_dlg_osstring(dialog, IDC_LITEX_KERNEL as i32)
                    .map_err(Error::WinError)
                    .map(|kpath| PathBuf::from(kpath));

                let boot_addr = get_dlg_osstring(dialog, IDC_LITEX_BOOT_ADDR as i32)
                    .map_err(Error::WinError)
                    .and_then(|os| {
                        let boot_str = os.to_string_lossy().into_owned();
                        parse::<u32>(&boot_str).map_err(|_| Error::BadAddressError(boot_str))
                    });

                debug!(target: "setup_dialog", "Kernel Path: {:?}", kernel_path);
                debug!(target: "setup_dialog", "Boot Address: {:?}", boot_addr);
                debug!(target: "setup_dialog", "Active: {:?}", active);

                TTX_LITEX_STATE.with_borrow_mut(|s| {
                    s.filename = kernel_path.ok();
                    s.addr = boot_addr.as_ref().copied().unwrap_or(0x40000000);

                    if s.filename.is_some() && boot_addr.is_ok() && active {
                        s.matcher.reset();
                        s.activity = Activity::LookForMagic;
                        s.last_frame_acked = None;
                        s.last_frame_sent = None;

                        info!(target: "setup_dialog", "Plugin now actively searching for magic string.");
                    } else {
                        s.activity = Activity::Inactive;
                    }
                });

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

ttx_export! {
    pub unsafe fn ttx_modify_menu(menu: HMENU) {
        let file_menu = GetSubMenu(menu, tt::ID_FILE as i32);
        // ID_TRANSFER == 9 in TeraTerm, and it doesn't work. Was the constant
        // never updated?
        let transfer_menu = GetSubMenu(file_menu, 11);

        let _ = AppendMenuW(
            transfer_menu,
            MF_ENABLED | MF_STRING,
            ID_MENU_LITEX,
            PCWSTR(u16cstr!("LiteX").as_ptr()),
        );
    }
}

ttx_export! {
    pub unsafe fn ttx_process_command(window: HWND, cmd: u16) -> i32 {
        match cmd as usize {
            ID_MENU_LITEX => {
                debug!(target: "TTXProcessCommand", "LiteX option clicked.");

                let res = DialogBoxParamW(
                    Some(OUR_HINST.get()),
                    PCWSTR(IDD_SETUP_LITEX as *const u16),
                    Some(window),
                    Some(Some(litex_setup_dialog)),
                    LPARAM(0),
                );

                if res <= 0 {
                    debug!(target: "TTXProcessCommand", "Could not open LiteX dialog: {}", Error::WinError(GetLastError().into()))
                }

                return 1;
            }
            _ => {
                return 0;
            }
        }
    }
}
