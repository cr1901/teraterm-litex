#[cfg(all(target_env = "msvc", target_pointer_width="32"))]
pub mod msvc_32;
#[cfg(all(target_env = "gnu", target_pointer_width="64"))]
pub mod gnu_64;

#[cfg(all(target_env = "msvc", target_pointer_width="32"))]
pub use msvc_32::*;
#[cfg(all(target_env = "gnu", target_pointer_width="64"))]
pub use gnu_64::*;

#[cfg(target_env = "msvc")]
#[macro_export]
macro_rules! ttx_export {
    ($(#[$m:meta])* $v:vis unsafe fn $f:ident $args:tt { $($rest:tt)* }) => { 
        $(#[$m])* $v unsafe extern "stdcall" fn $f $args { $($rest)* }
    };
    ($(#[$m:meta])* $v:vis unsafe fn $f:ident $args:tt -> $ret:ty { $($rest:tt)* }) => { 
        $(#[$m])* $v unsafe extern "stdcall" fn $f $args -> $ret { $($rest)* }
    };
}

// https://stackoverflow.com/a/44710341
// https://stackoverflow.com/a/54938004
#[cfg(target_env = "gnu")]
macro_rules! ttx_export {
    ($(#[$m:meta])* $v:vis unsafe fn $f:ident $args:tt { $($rest:tt)* }) => { 
        $(#[$m])* $v unsafe extern "C" fn $f $args { $($rest)* }
    };
    ($(#[$m:meta])* $v:vis unsafe fn $f:ident $args:tt -> $ret:ty { $($rest:tt)* }) => { 
        $(#[$m])* $v unsafe extern "C" fn $f $args -> $ret { $($rest)* }
    };
}
