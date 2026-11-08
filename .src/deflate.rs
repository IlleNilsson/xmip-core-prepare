//! Deflate (RFC 1951) both ways, over a compressor or decompressor a step
//! keeps (`crate::kept`) and resets for each Stream. zlib's frame (RFC
//! 1950) is the compressor's own; gzip's (`crate::gzip`) and a zip entry's
//! (`crate::zip`) are written around the raw stream, each checked by the
//! CRC-32 here.
//!
//! flate2's readers and writers build their compressor for every Stream:
//! about 200 KiB of tables made and zeroed to compress 1 KiB, 160 µs of a
//! Stream's 170 (2026-11-05). Driven directly, the one compressor a step
//! keeps is reset instead.

use std::io::{self, BufRead, Read, Write};

use flate2::{Compress, Crc, Decompress, FlushCompress, FlushDecompress, Status};

use crate::limit::Tally;

/// Compress all of `input` into `output`, read through `read` and written
/// through `written`.
///
/// # Errors
/// Reading or writing failed.
pub(crate) fn deflate(
    compress: &mut Compress,
    read: &mut [u8],
    written: &mut [u8],
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> io::Result<()> {
    compress.reset();
    loop {
        let held = input.read(read)?;
        let finishing = held == 0;
        let flush = if finishing {
            FlushCompress::Finish
        } else {
            FlushCompress::None
        };
        let mut rest = &read[..held];
        loop {
            let (before_in, before_out) = (compress.total_in(), compress.total_out());
            let status = compress
                .compress(rest, written, flush)
                .map_err(io::Error::other)?;
            let used = moved(before_in, compress.total_in());
            let made = moved(before_out, compress.total_out());
            output.write_all(&written[..made])?;
            rest = &rest[used..];
            if status == Status::StreamEnd {
                return Ok(());
            }
            // More input is wanted once this piece is in and nothing more
            // waits to be written; finishing goes on to the stream's end.
            if !finishing && rest.is_empty() && made < written.len() {
                break;
            }
        }
    }
}

/// Decompress one deflate stream from `input` into `output`, leaving in
/// `input` whatever follows the stream; how much it held. Every piece is
/// counted against `tally` before it is written.
///
/// # Errors
/// The stream is not deflate, ends before its end, makes more than
/// `tally`'s limit allows, or reading or writing failed.
pub(crate) fn inflate(
    decompress: &mut Decompress,
    zlib: bool,
    (written, tally): (&mut [u8], &Tally),
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> io::Result<u64> {
    decompress.reset(zlib);
    let mut total = 0u64;
    loop {
        let piece = input.fill_buf()?;
        let ended = piece.is_empty();
        let (before_in, before_out) = (decompress.total_in(), decompress.total_out());
        let status = decompress
            .decompress(piece, written, FlushDecompress::None)
            .map_err(|refused| io::Error::new(io::ErrorKind::InvalidData, refused))?;
        let used = moved(before_in, decompress.total_in());
        let made = moved(before_out, decompress.total_out());
        input.consume(used);
        tally.add(used, made)?;
        output.write_all(&written[..made])?;
        total += made as u64;
        if status == Status::StreamEnd {
            return Ok(total);
        }
        if ended && made == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the compressed data ends before its end",
            ));
        }
    }
}

/// How far a running total moved within one call: at most a buffer's
/// length.
fn moved(before: u64, after: u64) -> usize {
    usize::try_from(after - before).unwrap_or(usize::MAX)
}

/// The bytes on their way out, checked as they pass: written to the
/// output where there is one, and only checked where there is none — a
/// zip entry passed over.
pub(crate) struct Checked<'a> {
    output: Option<&'a mut dyn Write>,
    pub(crate) crc: Crc,
}

impl<'a> Checked<'a> {
    pub(crate) fn new(output: Option<&'a mut dyn Write>) -> Self {
        Self {
            output,
            crc: Crc::new(),
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

/// The bytes read, checked and counted as they pass.
pub(crate) struct CrcReader<'a> {
    pub(crate) inner: &'a mut dyn Read,
    pub(crate) crc: Crc,
    pub(crate) read: u64,
}

impl<'a> CrcReader<'a> {
    pub(crate) fn new(inner: &'a mut dyn Read) -> Self {
        Self {
            inner,
            crc: Crc::new(),
            read: 0,
        }
    }
}

impl Read for CrcReader<'_> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(into)?;
        self.crc.update(&into[..read]);
        self.read += read as u64;
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcore::kept::Reading;

    #[test]
    fn a_kept_compressor_carries_stream_after_stream() {
        let mut compress = Compress::new(flate2::Compression::default(), false);
        let mut decompress = Decompress::new(false);
        let (mut read, mut written) = (vec![0u8; 64], vec![0u8; 64]);
        for text in [&b"one, and one again, and one more"[..], b"two", b""] {
            let mut packed = Vec::new();
            deflate(
                &mut compress,
                &mut read,
                &mut written,
                &mut &text[..],
                &mut packed,
            )
            .expect("deflated");
            packed.extend_from_slice(b"after");
            let mut source = &packed[..];
            let mut reading = Reading::new(&mut source, &mut read);
            let mut back = Vec::new();
            let tally = Tally::new(crate::limit::Limit::new(1 << 20, 1));
            inflate(
                &mut decompress,
                false,
                (&mut written, &tally),
                &mut reading,
                &mut back,
            )
            .expect("inflated");
            assert_eq!(back, text);
            let mut after = Vec::new();
            reading.read_to_end(&mut after).expect("what follows");
            assert_eq!(after, b"after", "what follows the stream is left");
        }
    }
}
