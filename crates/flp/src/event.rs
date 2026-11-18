use std::path::Path;
use thiserror::Error;

/// Event id ranges. The id of an event determines the size of its payload.
pub const BYTE: u8 = 0;
pub const WORD: u8 = 64;
pub const DWORD: u8 = 128;
pub const TEXT: u8 = 192;
pub const DATA: u8 = 208;

#[derive(Debug, Error)]
pub enum FlpError {
    #[error("not an FL Studio project (missing FLhd header)")]
    BadMagic,
    #[error("unexpected end of file")]
    Truncated,
    #[error("unsupported header: {0}")]
    BadHeader(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventValue {
    Byte(u8),
    Word(u16),
    Dword(u32),
    Blob(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub id: u8,
    pub value: EventValue,
}

impl Event {
    pub fn new(id: u8, value: EventValue) -> Self {
        Self { id, value }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self.value {
            EventValue::Byte(v) => Some(v as u32),
            EventValue::Word(v) => Some(v as u32),
            EventValue::Dword(v) => Some(v),
            EventValue::Blob(_) => None,
        }
    }

    pub fn blob(&self) -> Option<&[u8]> {
        match &self.value {
            EventValue::Blob(b) => Some(b),
            _ => None,
        }
    }
}

/// A parsed `.flp` file: header fields plus the flat event stream.
#[derive(Clone, Debug)]
pub struct FlpFile {
    pub format: i16,
    pub channel_count: u16,
    pub ppq: u16,
    pub events: Vec<Event>,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], FlpError> {
        let end = self.pos.checked_add(n).ok_or(FlpError::Truncated)?;
        let s = self.buf.get(self.pos..end).ok_or(FlpError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, FlpError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, FlpError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, FlpError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn varint(&mut self) -> Result<usize, FlpError> {
        let mut n = 0usize;
        let mut shift = 0;
        loop {
            let b = self.u8()?;
            n |= ((b & 0x7f) as usize) << shift;
            if b & 0x80 == 0 {
                return Ok(n);
            }
            shift += 7;
            if shift > 28 {
                return Err(FlpError::BadHeader("invalid length prefix".into()));
            }
        }
    }
}

fn write_varint(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
}

impl FlpFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, FlpError> {
        let mut r = Reader { buf: bytes, pos: 0 };
        if r.take(4).map_err(|_| FlpError::BadMagic)? != b"FLhd" {
            return Err(FlpError::BadMagic);
        }
        let header_len = r.u32()?;
        if header_len != 6 {
            return Err(FlpError::BadHeader(format!("header length {header_len}")));
        }
        let format = r.u16()? as i16;
        let channel_count = r.u16()?;
        let ppq = r.u16()?;
        if r.take(4)? != b"FLdt" {
            return Err(FlpError::BadHeader("missing FLdt chunk".into()));
        }
        let data_len = r.u32()? as usize;
        // Some writers get the chunk size wrong; never read past the buffer.
        let end = r.pos.saturating_add(data_len).min(bytes.len());

        let mut events = Vec::new();
        while r.pos < end {
            let id = r.u8()?;
            let value = match id {
                0..=63 => EventValue::Byte(r.u8()?),
                64..=127 => EventValue::Word(r.u16()?),
                128..=191 => EventValue::Dword(r.u32()?),
                _ => {
                    let n = r.varint()?;
                    EventValue::Blob(r.take(n)?.to_vec())
                }
            };
            events.push(Event { id, value });
        }
        Ok(Self { format, channel_count, ppq, events })
    }

    pub fn read(path: impl AsRef<Path>) -> Result<Self, FlpError> {
        Self::parse(&std::fs::read(path)?)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = Vec::new();
        for e in &self.events {
            data.push(e.id);
            match &e.value {
                EventValue::Byte(v) => data.push(*v),
                EventValue::Word(v) => data.extend_from_slice(&v.to_le_bytes()),
                EventValue::Dword(v) => data.extend_from_slice(&v.to_le_bytes()),
                EventValue::Blob(b) => {
                    write_varint(&mut data, b.len());
                    data.extend_from_slice(b);
                }
            }
        }
        let mut out = Vec::with_capacity(data.len() + 22);
        out.extend_from_slice(b"FLhd");
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&self.format.to_le_bytes());
        out.extend_from_slice(&self.channel_count.to_le_bytes());
        out.extend_from_slice(&self.ppq.to_le_bytes());
        out.extend_from_slice(b"FLdt");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), FlpError> {
        Ok(std::fs::write(path, self.to_bytes())?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let f = FlpFile {
            format: 0,
            channel_count: 1,
            ppq: 96,
            events: vec![
                Event::new(9, EventValue::Byte(1)),
                Event::new(WORD, EventValue::Word(0)),
                Event::new(DWORD + 28, EventValue::Dword(140_000)),
                Event::new(TEXT + 7, EventValue::Blob(b"20.8.4.2576\0".to_vec())),
                Event::new(DATA + 16, EventValue::Blob(vec![7; 300])),
            ],
        };
        let bytes = f.to_bytes();
        let g = FlpFile::parse(&bytes).unwrap();
        assert_eq!(g.events, f.events);
        assert_eq!(g.to_bytes(), bytes);
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(FlpFile::parse(b"RIFF...."), Err(FlpError::BadMagic)));
    }
}
