//! One entry of a zip archive (PKWARE's APPNOTE), read and written as a
//! stream: read from its local header without the central directory,
//! which lies at the end, and written with its sizes after its data, so
//! neither direction holds the archive or seeks in it.
//!
//! Stored and deflated entries are read; an entry is written deflated.
//! An archive is one Stream's container, so an entry is one Stream: the
//! one named, or the first. Reading every entry as a Stream of its own is
//! debatching, not Preparation. A stored entry whose size is said only
//! after its data cannot be read as a stream and is refused, as is
//! writing an entry over 4 GiB, which needs zip64's records.

use std::io::{self, BufRead, Read, Write};

use flate2::Compression as Level;

use crate::{PrepareError, copy};

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const DESCRIPTOR: u32 = 0x0807_4b50;
const END: u32 = 0x0605_4b50;
/// Sizes and the check said after the data (APPNOTE 4.4.4, bit 3).
const SIZES_AFTER: u16 = 1 << 3;
/// The name is UTF-8 (APPNOTE 4.4.4, bit 11).
const UTF8_NAME: u16 = 1 << 11;
const STORED: u16 = 0;
const DEFLATED: u16 = 8;
/// 1980-01-01, the first date the archive's clock has.
const FIRST_DATE: u16 = (1 << 5) | 1;
/// APPNOTE 4.4.3: version 2.0 reads deflate and the data descriptor.
const VERSION: u16 = 20;
/// The zip64 extended information's header ID (APPNOTE 4.5.2).
const ZIP64: u16 = 0x0001;

const NAME: &str = crate::declaration::COMPRESSION;

fn refused(why: impl std::fmt::Display) -> PrepareError {
    PrepareError::new(format!("{NAME}: {why}"))
}

/// A local file header, as far as reading its entry needs.
struct Local {
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u64,
    name: String,
    zip64: bool,
}

/// Write the entry `wanted` names, or the first, from the archive `input`
/// to `output`, its check verified.
///
/// # Errors
/// The archive ends before such an entry, an entry is neither stored nor
/// deflated, its data does not match its check, or it cannot be read.
pub(crate) fn read_entry(
    mut input: impl BufRead,
    wanted: Option<&str>,
    output: &mut dyn Write,
) -> Result<u64, PrepareError> {
    loop {
        let signature = u32_le(&mut input)?;
        if signature != LOCAL {
            return Err(match wanted {
                Some(name) => refused(format!("the archive holds no entry named '{name}'")),
                None => refused("the archive holds no entry"),
            });
        }
        let local = local_header(&mut input)?;
        let chosen = wanted.is_none_or(|name| name == local.name);
        let mut checked = Checked::new(if chosen { Some(&mut *output) } else { None });
        let written = match local.method {
            STORED if local.flags & SIZES_AFTER != 0 => {
                return Err(refused(format!(
                    "the stored entry '{}' says its size only after its data",
                    local.name
                )));
            }
            STORED => copy(NAME, &mut (&mut input).take(local.compressed), &mut checked)?,
            DEFLATED => {
                let mut decoder = flate2::bufread::DeflateDecoder::new(&mut input);
                copy(NAME, &mut decoder, &mut checked)?
            }
            other => {
                return Err(refused(format!(
                    "the entry '{}' is compressed by method {other}, neither stored nor deflated",
                    local.name
                )));
            }
        };
        let expected = if local.flags & SIZES_AFTER == 0 {
            local.crc
        } else {
            descriptor(&mut input, local.zip64)?
        };
        if chosen {
            if checked.crc.sum() != expected {
                return Err(refused(format!(
                    "the entry '{}' does not match its check",
                    local.name
                )));
            }
            return Ok(written);
        }
    }
}

fn local_header(input: &mut impl Read) -> Result<Local, PrepareError> {
    let mut fixed = [0u8; 26];
    read_exact(input, &mut fixed)?;
    let field16 = |at: usize| u16::from_le_bytes([fixed[at], fixed[at + 1]]);
    let field32 =
        |at: usize| u32::from_le_bytes([fixed[at], fixed[at + 1], fixed[at + 2], fixed[at + 3]]);
    let mut name = vec![0u8; usize::from(field16(22))];
    read_exact(input, &mut name)?;
    let mut extra = vec![0u8; usize::from(field16(24))];
    read_exact(input, &mut extra)?;
    let stated = u64::from(field32(14));
    let zip64 = zip64_size(&extra, field32(18), stated);
    Ok(Local {
        flags: field16(2),
        method: field16(4),
        crc: field32(10),
        compressed: zip64.unwrap_or(stated),
        name: String::from_utf8_lossy(&name).into_owned(),
        zip64: zip64.is_some(),
    })
}

/// The compressed size the zip64 extended information gives, where the
/// header carries one (APPNOTE 4.5.3: the original size first, where its
/// field is full, then the compressed size).
fn zip64_size(extra: &[u8], original: u32, compressed: u64) -> Option<u64> {
    let mut rest = extra;
    while rest.len() >= 4 {
        let id = u16::from_le_bytes([rest[0], rest[1]]);
        let size = usize::from(u16::from_le_bytes([rest[2], rest[3]]));
        let body = rest.get(4..4 + size)?;
        if id == ZIP64 {
            let skip = if original == u32::MAX { 8 } else { 0 };
            let field = body.get(skip..skip + 8);
            return Some(match (compressed == u64::from(u32::MAX), field) {
                (true, Some(field)) => u64::from_le_bytes(field.try_into().ok()?),
                _ => compressed,
            });
        }
        rest = &rest[4 + size..];
    }
    None
}

/// The check a data descriptor gives, its signature optional and its sizes
/// eight bytes wide after a zip64 header (APPNOTE 4.3.9).
fn descriptor(input: &mut impl Read, zip64: bool) -> Result<u32, PrepareError> {
    let first = u32_le(input)?;
    let crc = if first == DESCRIPTOR {
        u32_le(input)?
    } else {
        first
    };
    let mut sizes = [0u8; 16];
    read_exact(input, &mut sizes[..if zip64 { 16 } else { 8 }])?;
    Ok(crc)
}

/// Write `input` to `output` as an archive of one entry called `name`,
/// deflated at `level`.
///
/// # Errors
/// The entry is over 4 GiB, or reading or writing failed.
pub(crate) fn write_entry(
    input: &mut dyn Read,
    name: &str,
    level: Level,
    output: &mut dyn Write,
) -> Result<(), PrepareError> {
    let mut counted = Counted {
        inner: output,
        written: 0,
    };
    let flags = SIZES_AFTER | UTF8_NAME;
    let mut header = Vec::with_capacity(30 + name.len());
    put32(&mut header, LOCAL);
    for field in [VERSION, flags, DEFLATED, 0, FIRST_DATE] {
        put16(&mut header, field);
    }
    header.extend_from_slice(&[0; 12]);
    put16(&mut header, name_length(name)?);
    put16(&mut header, 0);
    header.extend_from_slice(name.as_bytes());
    write_all(&mut counted, &header)?;

    let mut crc = flate2::Crc::new();
    let mut original = 0u64;
    let mut reading = CrcReader {
        inner: input,
        crc: &mut crc,
        read: &mut original,
    };
    let before = counted.written;
    let mut encoder = flate2::write::DeflateEncoder::new(&mut counted, level);
    copy(NAME, &mut reading, &mut encoder)?;
    encoder
        .finish()
        .map_err(|failed| PrepareError::of_io(NAME, &failed))?;
    let compressed = counted.written - before;
    let (compressed, original) = match (u32::try_from(compressed), u32::try_from(original)) {
        (Ok(compressed), Ok(original)) if original != u32::MAX && compressed != u32::MAX => {
            (compressed, original)
        }
        _ => {
            return Err(refused(
                "an entry over 4 GiB is not written; name gzip or zstd",
            ));
        }
    };

    let mut trailer = Vec::with_capacity(16 + 46 + name.len() + 22);
    for field in [DESCRIPTOR, crc.sum(), compressed, original] {
        put32(&mut trailer, field);
    }
    let central_at = counted.written + trailer.len() as u64;
    put32(&mut trailer, CENTRAL);
    for field in [VERSION, VERSION, flags, DEFLATED, 0, FIRST_DATE] {
        put16(&mut trailer, field);
    }
    for field in [crc.sum(), compressed, original] {
        put32(&mut trailer, field);
    }
    put16(&mut trailer, name_length(name)?);
    trailer.extend_from_slice(&[0; 12]);
    put32(&mut trailer, 0);
    trailer.extend_from_slice(name.as_bytes());
    let central_size = counted.written + trailer.len() as u64 - central_at;
    put32(&mut trailer, END);
    for field in [0, 0, 1, 1] {
        put16(&mut trailer, field);
    }
    for field in [central_size, central_at] {
        put32(
            &mut trailer,
            u32::try_from(field).map_err(|_| refused("the archive is over 4 GiB"))?,
        );
    }
    put16(&mut trailer, 0);
    write_all(&mut counted, &trailer)
}

fn name_length(name: &str) -> Result<u16, PrepareError> {
    u16::try_from(name.len()).map_err(|_| refused("an entry's name is over 65535 bytes"))
}

fn put16(into: &mut Vec<u8>, value: u16) {
    into.extend_from_slice(&value.to_le_bytes());
}

fn put32(into: &mut Vec<u8>, value: u32) {
    into.extend_from_slice(&value.to_le_bytes());
}

fn u32_le(input: &mut impl Read) -> Result<u32, PrepareError> {
    let mut bytes = [0u8; 4];
    read_exact(input, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_exact(input: &mut impl Read, into: &mut [u8]) -> Result<(), PrepareError> {
    input
        .read_exact(into)
        .map_err(|failed| match failed.kind() {
            io::ErrorKind::UnexpectedEof => refused("the archive ends inside an entry"),
            _ => PrepareError::of_io(NAME, &failed),
        })
}

fn write_all(output: &mut impl Write, bytes: &[u8]) -> Result<(), PrepareError> {
    output
        .write_all(bytes)
        .map_err(|failed| PrepareError::of_io(NAME, &failed))
}

/// The entry's bytes on their way out, checked as they pass: written to
/// the output where the entry is the one chosen, and only checked where it
/// is passed over.
struct Checked<'a> {
    output: Option<&'a mut dyn Write>,
    crc: flate2::Crc,
}

impl<'a> Checked<'a> {
    fn new(output: Option<&'a mut dyn Write>) -> Self {
        Self {
            output,
            crc: flate2::Crc::new(),
        }
    }
}

impl Write for Checked<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = match self.output.as_mut() {
            Some(output) => output.write(bytes)?,
            None => bytes.len(),
        };
        self.crc.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.as_mut().map_or(Ok(()), Write::flush)
    }
}

/// The bytes written, counted.
struct Counted<'a> {
    inner: &'a mut dyn Write,
    written: u64,
}

impl Write for Counted<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.inner.write(bytes)?;
        self.written += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// The bytes read, checked as they pass.
struct CrcReader<'a> {
    inner: &'a mut dyn Read,
    crc: &'a mut flate2::Crc,
    read: &'a mut u64,
}

impl Read for CrcReader<'_> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(into)?;
        self.crc.update(&into[..read]);
        *self.read += read as u64;
        Ok(read)
    }
}
