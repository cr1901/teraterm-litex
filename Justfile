# --raw-line='use windows::{ Win32::Foundation::*, Win32::System::SystemServices::*, };' \

bindings TT_ROOT:
    bindgen -o src/teraterm/gnu_64.rs wrapper.h \
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

# This is what is deployed, seeing that the release builds are 32-bit and built
# against the MSVC compiler. OTOH I develop against 64-bit GNU.
# Must be run from a MSVC Developer Command Prompt if your bindgen is 64-bits,
# x86 Native Tools Command Prompt is probably fine if your bindgen is 32-bits,
# but I haven't tested.
bindings-32-msvc TT_ROOT:
    bindgen -o src/teraterm/msvc_32.rs wrapper.h \
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
        -- -I {{TT_ROOT}}/teraterm/teraterm/ -I {{TT_ROOT}}/teraterm/common --target=i686-pc-windows-msvc

# Requires "rustup toolchain install stable-i686-msvc".
# Must be from from a x86 Native Tools Command Prompt.
build-32-msvc:
    cargo +stable-i686-msvc build --release --target i686-pc-windows-msvc
