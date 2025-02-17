# --raw-line='use windows::{ Win32::Foundation::*, Win32::System::SystemServices::*, };' \

# Install 64-bit GNU bindings.
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
#
# Alternatively, Bindgen/Clang might be able to find windows.h and friends if
# MSVC headers are on the PATH, appropriate env vars are set, and/or
# vcvarsall.bat has been run. But I also have not tested this. I was annoyed
# enough that I had to install MSVC :).
# Install 32-bit MSVC bindings.
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

# Copy DLL to built TeraTerm for testing.
build-cp TT_ROOT:
    cargo build
    cp target/debug/TTXLiteX.dll {{TT_ROOT}}/build

# Format and Fix source code.
fmt-fix:
    cargo fmt
    cargo fix --allow-dirty
    git commit -am "cargo fmt. cargo fix."

# Install the 32-bit MSVC Rust compiler, required for releases.
prereq-32-msvc:
    rustup toolchain install stable-i686-msvc

# Requires "rustup toolchain install stable-i686-msvc".
# Must be from from a x86 Native Tools Command Prompt, because otherwise the
# GNU Resource Compiler might be called during the build script.
# Build release DLL for 32-bit MSVC, which matches TeraTerm releases.
build-32-msvc:
    cargo +stable-i686-msvc build --release --target i686-pc-windows-msvc

# Initialize installer XML.
wix-init-32-msvc:
    cargo +stable-i686-msvc wix init --force --license wix/License.rtf

# Create installers for TeraTerm 4 and 5.
wix-msi-32-msvc VERSION="":
    cargo +stable-i686-msvc wix -C '-dTTX_LITEX_TERATERM5=1' --target i686-pc-windows-msvc -vo target/i686-pc-windows-msvc/wix/tt5-litex{{VERSION}}-i686.msi
    cargo +stable-i686-msvc wix --target i686-pc-windows-msvc -vo target/i686-pc-windows-msvc/wix/tt-litex{{VERSION}}-i686.msi

# Create zip file of release assets.
prepare-release-zip VERSION="":
    rm -rf tt-litex{{VERSION}}
    mkdir tt-litex{{VERSION}}
    mkdir tt-litex{{VERSION}}/assets
    cp CHANGELOG.md tt-litex{{VERSION}}
    cp README.md tt-litex{{VERSION}}
    cp LICENSE.md tt-litex{{VERSION}}
    cp assets/*.* tt-litex{{VERSION}}/assets
    cp target/i686-pc-windows-msvc/wix/*.msi tt-litex{{VERSION}}
    cp target/i686-pc-windows-msvc/release/TTXLiteX.dll tt-litex{{VERSION}}
    zip -r tt-litex{{VERSION}}.zip tt-litex{{VERSION}}/*
