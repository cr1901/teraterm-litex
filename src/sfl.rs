/*! Serial Flash Loader implementation. */

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::mem::offset_of;
use std::path::Path;

use crc;
use zerocopy::{byteorder::big_endian::U16, Immutable, IntoBytes};

const CCITT: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);
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

pub struct SflLoader<R> {
    reader: R,
    base: u32,
    #[allow(unused)]
    offs: usize,
    pub chunk_size: u16,
}

impl SflLoader<File> {
    pub fn open<P>(path: P, base: u32) -> Result<SflLoader<File>, io::Error>
    where
        P: AsRef<Path>,
    {
        Ok(SflLoader::new(File::open(path)?, base))
    }
}

impl<R> SflLoader<R> {
    pub fn new(reader: R, base: u32) -> Self {
        Self {
            reader,
            base,
            offs: 0,
            chunk_size: 251,
        }
    }

    pub fn halve_chunk_size(&mut self) {
        if self.chunk_size == 251 {
            self.chunk_size = 128;
        }
        else if self.chunk_size > 16 {
            self.chunk_size /= 2;
        }
    }

    pub fn encode_data_frame(&mut self, frame_num: u32) -> Result<Option<Box<Frame>>, io::Error>
    where
        R: Read + Seek,
    {
        let mut frame = Box::new(Frame {
            len: 0,
            crc: 0.into(),
            cmd: Cmd::Load,
            payload: [0; 255],
        });

        let addr = self.base + frame_num * (self.chunk_size as u32);

        let addr_be = addr.to_be_bytes();
        frame.payload[0..4].copy_from_slice(&addr_be);
        frame.len = 4;

        // XXX: This will seek past the end on last iteration. Works fine on
        // Windows, but should probably be careful.
        self.reader.seek(SeekFrom::Start(
            (frame_num * (self.chunk_size as u32)).into(),
        ))?;
        let read_len = self
            .reader
            .read(&mut frame.payload[4..((self.chunk_size + 4) as usize)])?;
        if read_len == 0 {
            return Ok(None);
        }
        frame.len += read_len as u8;

        let crc = CCITT.checksum(&frame.as_bytes()[offset_of!(Frame, cmd)..]);
        frame.crc = crc.into();

        Ok(Some(frame))
    }

    pub fn encode_boot_frame(&mut self, address: u32) -> Box<Frame> {
        let mut frame = Box::new(Frame {
            len: 0,
            crc: 0.into(),
            cmd: Cmd::Jump,
            payload: [0; 255],
        });

        let addr_be = address.to_be_bytes();
        frame.payload[0..4].copy_from_slice(&addr_be);
        frame.len = 4;

        let crc = CCITT.checksum(&frame.as_bytes()[offset_of!(Frame, cmd)..]);
        frame.crc = crc.into();

        frame
    }
}

#[derive(IntoBytes, Immutable, Debug)]
#[repr(packed)]
#[repr(C)]
pub struct Frame {
    len: u8,
    crc: U16,
    cmd: Cmd,
    payload: [u8; 255],
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
