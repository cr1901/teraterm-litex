/*! Plugin Error type. */

use std::{fmt, io};

#[derive(Debug)]
pub enum Error {
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
