//! Compression: on receive a compressed Stream is decompressed, on send a
//! Stream is compressed, each a piece at a time. The format is a setting,
//! never the unit's name (the owner, 2026-10-21).
//!
//! gzip, zlib, raw deflate and a zip entry are deflate (`crate::deflate`),
//! over `flate2`'s pure-Rust backend, each in its own frame: zlib's the
//! compressor's, gzip's `crate::gzip`, a zip archive's `crate::zip`, one
//! entry being one Stream. zstd is `ruzstd`'s, Rust throughout, where the
//! `zstd` crate binds the C library.
//!
//! A step keeps its compressor or decoder between Streams (`crate::kept`),
//! all but zstd's compressor, which `ruzstd` makes for every Stream and
//! gives no way to keep.

use std::io::{Read, Write};

use flate2::{Compress, Compression as Level, Decompress};
use ruzstd::decoding::{FrameDecoder, StreamingDecoder};

use crate::declaration::{COMPRESSION as NAME, COMPRESSION_UNIT};
use crate::deflate::{deflate, inflate};
use crate::direction::Direction;
use crate::{CHUNK, Prepare, PrepareError, PrepareUnit, Stage, Values, copy, gzip, zip};
use xcore::kept::Kept;
use xcore::kept::Reading;

/// Compression, as the standard module declares it.
pub const UNIT: PrepareUnit = PrepareUnit::new(COMPRESSION_UNIT, open);

/// A compression format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Gzip,
    Zlib,
    Deflate,
    Zstd,
    Zip,
}

/// Compression opened for one step.
struct Compression {
    format: Format,
    level: Level,
    entry: Option<String>,
    /// Its coder, the one its format and direction need.
    kept: Kept<Working>,
}

/// What a step compresses or decompresses with, kept between Streams.
struct Working {
    read: Vec<u8>,
    written: Vec<u8>,
    coder: Coder,
}

/// The one coder a step's format and direction need.
enum Coder {
    Deflate(Compress),
    Inflate(Decompress),
    Frames(Box<FrameDecoder>),
    /// zstd's compressor, made by `ruzstd` for every Stream.
    Made,
}

impl Working {
    fn new(format: Format, level: Level, direction: Direction) -> Self {
        let zlib = format == Format::Zlib;
        let coder = match (format, direction) {
            (Format::Zstd, Direction::Inward) => Coder::Frames(Box::new(FrameDecoder::new())),
            (Format::Zstd, Direction::Outward) => Coder::Made,
            (_, Direction::Inward) => Coder::Inflate(Decompress::new(zlib)),
            (_, Direction::Outward) => Coder::Deflate(Compress::new(level, zlib)),
        };
        Self {
            read: vec![0u8; CHUNK],
            written: vec![0u8; CHUNK],
            coder,
        }
    }
}

fn open(stage: Stage, values: &Values) -> Result<Box<dyn Prepare>, String> {
    let format = match values.text("format") {
        "gzip" => Format::Gzip,
        "zlib" => Format::Zlib,
        "deflate" => Format::Deflate,
        "zstd" => Format::Zstd,
        _ => Format::Zip,
    };
    let level = match values.text("level") {
        "fastest" => Level::fast(),
        "best" => Level::best(),
        _ => Level::default(),
    };
    let direction = Direction::of(NAME, stage, values, ["decompress", "compress"])?;
    Ok(Box::new(Compression {
        format,
        level,
        entry: values.optional_text("entry").map(str::to_string),
        kept: Kept::new(move || Working::new(format, level, direction)),
    }))
}

impl Prepare for Compression {
    fn prepare(
        &self,
        input: &mut (dyn Read + Send),
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        self.kept.with(|working| match &mut working.coder {
            Coder::Deflate(compress) => self.deflate(
                compress,
                (&mut working.read, &mut working.written),
                input,
                output,
            ),
            Coder::Inflate(decompress) => self.inflate(
                decompress,
                (&mut working.read, &mut working.written),
                input,
                output,
            ),
            Coder::Frames(frames) => {
                let mut reading = Reading::new(input, &mut working.read);
                let mut decoder =
                    StreamingDecoder::new_with_decoder(&mut reading, &mut **frames)
                        .map_err(|refused| PrepareError::new(format!("{NAME}: {refused}")))?;
                copy(NAME, &mut decoder, output)?;
                output
                    .flush()
                    .map_err(|failed| PrepareError::of_io(NAME, &failed))
            }
            // ruzstd compresses at its fastest level alone; its others are
            // declared and not implemented, and panic. The declaration says
            // so of `level`.
            Coder::Made => {
                ruzstd::encoding::compress(
                    input,
                    output,
                    ruzstd::encoding::CompressionLevel::Fastest,
                );
                Ok(())
            }
        })
    }
}

impl Compression {
    fn deflate(
        &self,
        compress: &mut Compress,
        (read, written): (&mut [u8], &mut [u8]),
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        let failed = |failed: std::io::Error| PrepareError::of_io(NAME, &failed);
        match self.format {
            Format::Gzip => {
                gzip::write_member(compress, self.level, (read, written), input, output)
            }
            Format::Zip => zip::write_entry(
                input,
                self.entry.as_deref().unwrap_or("content"),
                (compress, read, written),
                output,
            ),
            _ => deflate(compress, read, written, input, output)
                .and_then(|()| output.flush())
                .map_err(failed),
        }
    }

    fn inflate(
        &self,
        decompress: &mut Decompress,
        (read, written): (&mut [u8], &mut [u8]),
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        let failed = |failed: std::io::Error| PrepareError::of_io(NAME, &failed);
        let mut reading = Reading::new(input, read);
        match self.format {
            // A gzip Stream may hold several members end to end (RFC 1952
            // section 2.2), read as one.
            Format::Gzip => gzip::read_members(decompress, written, &mut reading, output),
            Format::Zip => zip::read_entry(
                &mut reading,
                self.entry.as_deref(),
                (decompress, written),
                output,
            )
            .map(|_| ()),
            zlib_or_deflate => inflate(
                decompress,
                zlib_or_deflate == Format::Zlib,
                written,
                &mut reading,
                output,
            )
            .and_then(|_| output.flush())
            .map_err(failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{prepared, text};

    const TEXT: &[u8] = b"<Order><Line>one</Line><Line>two</Line></Order>\n";

    fn both_ways(format: &str) {
        let given = [("format", text(format))];
        let compressed = prepared(NAME, Stage::Send, &given, TEXT).expect("compressed");
        assert_ne!(compressed, TEXT, "{format}");
        let back = prepared(NAME, Stage::Receive, &given, &compressed).expect("back");
        assert_eq!(back, TEXT, "{format}");
    }

    #[test]
    fn every_format_compresses_on_send_and_decompresses_on_receive() {
        for format in crate::declaration::COMPRESSION_FORMATS {
            both_ways(format);
        }
    }

    #[test]
    fn process_preparation_names_its_direction() {
        let refused = prepared(NAME, Stage::Process, &[], TEXT).expect_err("needs one");
        assert!(
            refused.to_string().contains("'decompress' or 'compress'"),
            "{refused}"
        );
        let compressed = prepared(
            NAME,
            Stage::Process,
            &[("direction", text("compress")), ("level", text("best"))],
            TEXT,
        )
        .expect("compressed");
        let back = prepared(
            NAME,
            Stage::Process,
            &[("direction", text("decompress"))],
            &compressed,
        )
        .expect("back");
        assert_eq!(back, TEXT);
    }

    #[test]
    fn what_is_not_compressed_as_named_is_refused_in_words() {
        for format in crate::declaration::COMPRESSION_FORMATS {
            let refused = prepared(NAME, Stage::Receive, &[("format", text(format))], TEXT)
                .expect_err("not compressed");
            assert!(
                refused.to_string().starts_with("Compression:"),
                "{format}: {refused}"
            );
        }
    }

    #[test]
    fn a_gzip_stream_of_two_members_reads_as_one() {
        let given = [("format", text("gzip"))];
        let mut two = prepared(NAME, Stage::Send, &given, b"one ").expect("first");
        two.extend(prepared(NAME, Stage::Send, &given, b"two").expect("second"));
        let back = prepared(NAME, Stage::Receive, &given, &two).expect("both");
        assert_eq!(back, b"one two");
    }
}
