use std::io::{self, Read};

use crc;
use zerocopy::{byteorder::big_endian::U16, Immutable, IntoBytes};

const CCITT: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_IBM_3740);
pub const MAGIC: &'static [u8] = b"sL5DdSMmkekro\n";
pub const MAGIC_RESPONSE: &'static [u8] = b"z6IHG7cYDID6o\n";

pub struct MagicMatcher {
    magic: &'static [u8],
    state: usize,
}

#[derive(IntoBytes, Immutable)]
#[repr(u8)]
pub enum Cmd {
    Abort = 0,
    Load = 1,
    Jump = 2
}

#[repr(u8)]
pub enum Resp {
    Success = b'K',
    CrcError = b'C',
    Unknown = b'U',
    AckError = b'E'
}

pub struct TryFromU8Error(());

impl TryFrom<u8> for Resp {
    type Error = TryFromU8Error;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            b'K' => { Ok(Resp::Success) },
            b'C' => { Ok(Resp::CrcError) },
            b'U' => { Ok(Resp::Unknown) },
            b'E' => { Ok(Resp::AckError) },
            _ => { Err(TryFromU8Error(())) }
        }
    }
}

#[derive(IntoBytes, Immutable)]
#[repr(packed)]
pub struct Frame {
    len: u8,
    crc: U16,
    cmd: Cmd,
    payload: [u8; 255]
}

pub fn encode_data_frame<R>(address: u32, reader: &mut R) -> Result<Box<Frame>, io::Error> where R: Read {
    let mut frame = Box::new(
        Frame {
            len: 0,
            crc: 0.into(),
            cmd: Cmd::Load,
            payload: [0; 255]
        }
    );

    let addr_be = address.to_be_bytes();
    frame.payload[0..4].copy_from_slice(&addr_be);
    frame.len = 4;

    let read_len = reader.read(&mut frame.payload[4..])?;
    frame.len += read_len as u8;

    let crc = CCITT.checksum(&frame.as_bytes()[3..]);
    frame.crc = crc.into();

    Ok(frame)
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
}
