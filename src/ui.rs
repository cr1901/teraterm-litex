/*! UI and Window-creation hooks. */

use std::ffi::OsString;
use std::fmt::Write;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use log::*;
use rfd::FileDialog;

use super::sfl::SflLoader;
use super::state::{get_hinst_var, with_state_var, Activity};
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

    if used_len == 0 {
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
            // * SetWindowLongPtr to avoid with_state_var (Dialog is modal).

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
                                s.last_frame_acked = None;
                                s.last_frame_sent = None;
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

pub unsafe extern "C" fn ttx_modify_menu(menu: HMENU) {
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

pub unsafe extern "C" fn ttx_process_command(window: HWND, cmd: u16) -> i32 {
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
