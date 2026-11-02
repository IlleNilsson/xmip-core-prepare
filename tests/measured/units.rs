//! What the prepare units are carried with: text made as it is read, a
//! sink that counts it, the settings given and the units carried. The
//! bounded-memory test (`tests/bounded.rs`) and the module's timing program
//! both drive it; shared, not copied.

use std::io::{self, Read, Write};

use xcore::settings::Given;

/// Text made as it is read, `length` bytes: lines of letters, each ended by
/// CR LF.
pub struct Made {
    pub at: u64,
    pub length: u64,
}

impl Read for Made {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let left = usize::try_from(self.length - self.at).unwrap_or(usize::MAX);
        let taken = out.len().min(left);
        for byte in &mut out[..taken] {
            *byte = match self.at % 64 {
                62 => b'\r',
                63 => b'\n',
                at => b'a' + u8::try_from(at % 26).unwrap_or_default(),
            };
            self.at += 1;
        }
        Ok(taken)
    }
}

/// What was written, counted and summed, never kept.
#[derive(Default)]
pub struct Sink {
    pub length: u64,
    pub sum: u64,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.length += bytes.len() as u64;
        for &byte in bytes {
            self.sum = self.sum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub fn given(settings: &[(&str, &str)]) -> Vec<(String, Given)> {
    settings
        .iter()
        .map(|(name, value)| {
            let value = value
                .parse()
                .map_or_else(|_| Given::Text((*value).to_string()), Given::Integer);
            ((*name).to_string(), value)
        })
        .collect()
}

/// The units and settings carried, each both ways.
pub const CARRIED: &[(&str, &[(&str, &str)])] = &[
    ("Compression", &[("format", "gzip")]),
    ("Compression", &[("format", "zlib")]),
    ("Compression", &[("format", "deflate")]),
    ("Compression", &[("format", "zstd")]),
    ("Compression", &[("format", "zip")]),
    ("CharacterEncoding", &[("encoding", "utf-16le")]),
    ("CharacterEncoding", &[("encoding", "windows-1252")]),
    ("TransferEncoding", &[("line_length", "76")]),
    ("TextNormalization", &[("line_ending", "lf")]),
];
