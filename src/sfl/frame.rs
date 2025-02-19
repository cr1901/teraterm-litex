/*! Serial Flash Loader frame. */

use std::fmt;

use zerocopy::{byteorder::big_endian::U16, Immutable, IntoBytes};

pub const MAGIC: &'static [u8] = b"sL5DdSMmkekro\n";
pub const MAGIC_RESPONSE: &'static [u8] = b"z6IHG7cYDID6o\n";

pub struct MagicMatcher {
    magic: &'static [u8],
    state: usize,
}

#[derive(IntoBytes, Immutable, Debug, Clone, Copy)]
#[repr(u8)]
pub enum Cmd {
    #[allow(unused)]
    Abort = 0,
    Load = 1,
    Jump = 2,
}

#[repr(u8)]
#[derive(Debug)]
pub enum Resp {
    Success = b'K',
    CrcError = b'C',
    Unknown = b'U',
    AckError = b'E',
}

pub struct TryFromU8Error(());

impl TryFrom<u8> for Resp {
    type Error = TryFromU8Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            b'K' => Ok(Resp::Success),
            b'C' => Ok(Resp::CrcError),
            b'U' => Ok(Resp::Unknown),
            b'E' => Ok(Resp::AckError),
            _ => Err(TryFromU8Error(())),
        }
    }
}

impl fmt::Display for Resp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Resp::Success => {
                write!(f, "Success")
            }
            Resp::CrcError => {
                write!(f, "CRC Error")
            }
            Resp::Unknown => {
                write!(f, "Unknown Error")
            }
            Resp::AckError => {
                write!(f, "ACK Error")
            }
        }
    }
}

#[derive(IntoBytes, Immutable, Debug)]
#[repr(packed)]
#[repr(C)]
pub struct Frame {
    pub(super) len: u8,
    pub(super) crc: U16,
    pub(super) cmd: Cmd,
    pub(super) payload: [u8; 255],
}

impl Frame {
    pub fn as_bytes(&self) -> &[u8] {
        &IntoBytes::as_bytes(self)[..(((self.len as usize) + 4) as usize)]
    }
}

impl MagicMatcher {
    pub fn new(magic: &'static [u8]) -> Self {
        Self { magic, state: 0 }
    }

    pub fn look_for_match(&mut self, chunk: &[u8]) -> bool {
        if self.magic.len() == 0 {
            return true;
        }

        let mut found = false;
        for b in chunk {
            if *b == self.magic[self.state as usize] {
                if (self.state + 1) >= self.magic.len() {
                    found = true;
                    self.state = 0;
                } else {
                    self.state += 1;
                }
                continue;
            } else {
                self.state = 0;
            }
        }

        return found;
    }

    pub fn reset(&mut self) {
        self.state = 0;
    }
}
