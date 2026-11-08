//! gzip's frame (RFC 1952) around a deflate stream (`crate::deflate`):
//! written as one member, read as every member the Stream holds end to end
//! (section 2.2), each checked by its CRC-32 and its length.

use std::io::{self, BufRead, Read, Write};

use flate2::{Compress, Compression as Level, Decompress};

use crate::PrepareError;
use crate::deflate::{Checked, CrcReader, deflate, inflate};
use crate::limit::Tally;

/// The most bytes a member's name or comment is read to: FNAME and
/// FCOMMENT end at a NUL (section 2.3.1), and a Stream without one would
/// otherwise be held whole looking for it.
const MOST_TEXT: u64 = 64 * 1024;

const NAME: &str = crate::declaration::COMPRESSION;

/// ID1, ID2 and CM 8, deflate (section 2.3.1).
const MAGIC: [u8; 3] = [0x1f, 0x8b, 8];
/// FLG's bits (section 2.3.1).
const HEADER_CRC: u8 = 1 << 1;
const EXTRA: u8 = 1 << 2;
const FILE_NAME: u8 = 1 << 3;
const COMMENT: u8 = 1 << 4;
/// OS 255: unknown.
const UNKNOWN_SYSTEM: u8 = 255;

fn refused(why: impl std::fmt::Display) -> PrepareError {
    PrepareError::new(format!("{NAME}: {why}"))
}

fn failed(failed: &io::Error) -> PrepareError {
    match failed.kind() {
        io::ErrorKind::UnexpectedEof => refused("the gzip Stream ends inside a member"),
        _ => PrepareError::of_io(NAME, failed),
    }
}

/// Write `input` to `output` as one gzip member, deflated by `compress`
/// at `level`.
///
/// # Errors
/// Reading or writing failed.
pub(crate) fn write_member(
    compress: &mut Compress,
    level: Level,
    (read, written): (&mut [u8], &mut [u8]),
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), PrepareError> {
    // XFL says which effort compressed it (section 2.3.1), as flate2 says.
    let effort = if level.level() >= Level::best().level() {
        2
    } else if level.level() <= Level::fast().level() {
        4
    } else {
        0
    };
    let header = [
        MAGIC[0],
        MAGIC[1],
        MAGIC[2],
        0,
        0,
        0,
        0,
        0,
        effort,
        UNKNOWN_SYSTEM,
    ];
    output.write_all(&header).map_err(|error| failed(&error))?;
    let mut reading = CrcReader::new(input);
    deflate(compress, read, written, &mut reading, output).map_err(|error| failed(&error))?;
    // ISIZE is the length modulo 2^32 (section 2.3.1).
    let length = u32::try_from(reading.read & u64::from(u32::MAX)).unwrap_or_default();
    let mut trailer = [0u8; 8];
    trailer[..4].copy_from_slice(&reading.crc.sum().to_le_bytes());
    trailer[4..].copy_from_slice(&length.to_le_bytes());
    output.write_all(&trailer).map_err(|error| failed(&error))?;
    output.flush().map_err(|error| failed(&error))
}

/// Write every member of the gzip Stream `input` to `output`, as one, all
/// of them counted against the one `tally`.
///
/// # Errors
/// The Stream is not gzip, a member does not match its check or length,
/// it ends inside a member, the members make more than `tally`'s limit
/// allows, or reading or writing failed.
pub(crate) fn read_members(
    decompress: &mut Decompress,
    (written, tally): (&mut [u8], &Tally),
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<(), PrepareError> {
    loop {
        header(input)?;
        let mut checked = Checked::new(Some(&mut *output));
        let length = inflate(decompress, false, (written, tally), input, &mut checked)
            .map_err(|error| failed(&error))?;
        let mut trailer = [0u8; 8];
        input
            .read_exact(&mut trailer)
            .map_err(|error| failed(&error))?;
        let [c0, c1, c2, c3, l0, l1, l2, l3] = trailer;
        if u32::from_le_bytes([c0, c1, c2, c3]) != checked.crc.sum() {
            return Err(refused("a gzip member does not match its check"));
        }
        if u32::from_le_bytes([l0, l1, l2, l3])
            != u32::try_from(length & u64::from(u32::MAX)).unwrap_or_default()
        {
            return Err(refused("a gzip member is not the length it says"));
        }
        if input.fill_buf().map_err(|error| failed(&error))?.is_empty() {
            return output.flush().map_err(|error| failed(&error));
        }
    }
}

/// Read past a member's header, refusing what is not one.
fn header(input: &mut dyn BufRead) -> Result<(), PrepareError> {
    let mut fixed = [0u8; 10];
    input
        .read_exact(&mut fixed)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => refused("the Stream is not gzip"),
            _ => failed(&error),
        })?;
    if fixed[..3] != MAGIC {
        return Err(refused("the Stream is not gzip"));
    }
    let flags = fixed[3];
    if flags & EXTRA != 0 {
        let mut length = [0u8; 2];
        input
            .read_exact(&mut length)
            .map_err(|error| failed(&error))?;
        skip(input, u64::from(u16::from_le_bytes(length)))?;
    }
    for field in [FILE_NAME, COMMENT] {
        if flags & field != 0 {
            let mut ended = Vec::new();
            Read::take(&mut *input, MOST_TEXT)
                .read_until(0, &mut ended)
                .map_err(|error| failed(&error))?;
            if ended.last() != Some(&0) {
                return Err(refused(if ended.len() as u64 == MOST_TEXT {
                    "a gzip member's name or comment runs past 64 KiB"
                } else {
                    "the gzip Stream ends inside a member"
                }));
            }
        }
    }
    if flags & HEADER_CRC != 0 {
        skip(input, 2)?;
    }
    Ok(())
}

fn skip(input: &mut dyn BufRead, length: u64) -> Result<(), PrepareError> {
    let skipped = io::copy(&mut Read::take(&mut *input, length), &mut io::sink())
        .map_err(|error| failed(&error))?;
    if skipped < length {
        return Err(refused("the gzip Stream ends inside a member"));
    }
    Ok(())
}
