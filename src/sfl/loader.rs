/*! Basic SFL Loader implementation. */

use super::frame::*;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::mem::offset_of;
use std::path::Path;

use crc;
const CCITT: crc::Crc<u16> = crc::Crc::<u16>::new(&crc::CRC_16_XMODEM);

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
        } else if self.chunk_size > 16 {
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


