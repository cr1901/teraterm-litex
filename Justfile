# --raw-line='use windows::{ Win32::Foundation::*, Win32::System::SystemServices::*, };' \


bindings TT_ROOT:
    bindgen -o src/teraterm.rs wrapper.h \
        --allowlist-file={{TT_ROOT}}/teraterm/common/teraterm.h \
        --allowlist-file={{TT_ROOT}}/teraterm/common/tttypes.h \
        --allowlist-file={{TT_ROOT}}/teraterm/common/ttplugin.h \
        --blocklist-type=HMENU \
        --blocklist-type=HMENU__ \
        --blocklist-type=HWND \
        --blocklist-type=HWND__ \
        --blocklist-type=_OVERLAPPED \
        --blocklist-type=LPOVERLAPPED \
        --raw-line='#![allow(unused, non_upper_case_globals, non_snake_case, non_camel_case_types)]' \
        --raw-line='use windows::Win32::UI::WindowsAndMessaging::HMENU;' \
        --raw-line='use windows::Win32::Foundation::HWND;' \
        --raw-line='use windows::Win32::System::IO::OVERLAPPED;' \
        --raw-line='pub type LPOVERLAPPED=*mut OVERLAPPED;' \
        -- -I {{TT_ROOT}}/teraterm/teraterm/ -I {{TT_ROOT}}/teraterm/common
