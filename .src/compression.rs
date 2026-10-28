//! Compression: on receive a compressed Stream is decompressed, on send a
//! Stream is compressed, each a piece at a time. The format is a setting,
//! never the unit's name (the owner, 2026-10-21).
//!
//! gzip, zlib and raw deflate are `flate2`'s, over its pure-Rust backend;
//! zstd is `ruzstd`'s, Rust throughout, where the `zstd` crate binds the C
//! library; a zip archive is read and written here (`crate::zip`), over
//! `flate2`, one entry being one Stream.

use std::io::{BufReader, Read, Write};

use flate2::Compression as Level;

use crate::declaration::{COMPRESSION as NAME, COMPRESSION_UNIT};
use crate::direction::Direction;
use crate::{Prepare, PrepareError, PrepareUnit, Stage, Values, copy, zip};

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

/// How hard compressing works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effort {
    Fastest,
    Default,
    Best,
}

/// Compression opened for one step.
struct Compression {
    format: Format,
    effort: Effort,
    entry: Option<String>,
    direction: Direction,
}

fn open(stage: Stage, values: &Values) -> Result<Box<dyn Prepare>, String> {
    let format = match values.text("format") {
        "gzip" => Format::Gzip,
        "zlib" => Format::Zlib,
        "deflate" => Format::Deflate,
        "zstd" => Format::Zstd,
        _ => Format::Zip,
    };
    let effort = match values.text("level") {
        "fastest" => Effort::Fastest,
        "best" => Effort::Best,
        _ => Effort::Default,
    };
    Ok(Box::new(Compression {
        format,
        effort,
        entry: values.optional_text("entry").map(str::to_string),
        direction: Direction::of(NAME, stage, values, ["decompress", "compress"])?,
    }))
}

impl Prepare for Compression {
    fn prepare(
        &self,
        input: &mut (dyn Read + Send),
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        match self.direction {
            Direction::Inward => self.decompress(input, output),
            Direction::Outward => self.compress(input, output),
        }
    }
}

impl Compression {
    fn decompress(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<(), PrepareError> {
        let buffered = BufReader::with_capacity(crate::CHUNK, input);
        match self.format {
            // A gzip Stream may hold several members end to end (RFC 1952
            // section 2.2), read as one.
            Format::Gzip => copy(
                NAME,
                &mut flate2::bufread::MultiGzDecoder::new(buffered),
                output,
            ),
            Format::Zlib => copy(
                NAME,
                &mut flate2::bufread::ZlibDecoder::new(buffered),
                output,
            ),
            Format::Deflate => copy(
                NAME,
                &mut flate2::bufread::DeflateDecoder::new(buffered),
                output,
            ),
            Format::Zstd => {
                let mut decoder = ruzstd::decoding::StreamingDecoder::new(buffered)
                    .map_err(|refused| PrepareError::new(format!("{NAME}: {refused}")))?;
                copy(NAME, &mut decoder, output)
            }
            Format::Zip => zip::read_entry(buffered, self.entry.as_deref(), output),
        }
        .map(|_| ())
    }

    fn compress(&self, input: &mut dyn Read, output: &mut dyn Write) -> Result<(), PrepareError> {
        let level = match self.effort {
            Effort::Fastest => Level::fast(),
            Effort::Default => Level::default(),
            Effort::Best => Level::best(),
        };
        let failed = |failed: std::io::Error| PrepareError::of_io(NAME, &failed);
        match self.format {
            Format::Gzip => {
                let mut encoder = flate2::write::GzEncoder::new(output, level);
                copy(NAME, input, &mut encoder)?;
                encoder.finish().map_err(failed)?;
            }
            Format::Zlib => {
                let mut encoder = flate2::write::ZlibEncoder::new(output, level);
                copy(NAME, input, &mut encoder)?;
                encoder.finish().map_err(failed)?;
            }
            Format::Deflate => {
                let mut encoder = flate2::write::DeflateEncoder::new(output, level);
                copy(NAME, input, &mut encoder)?;
                encoder.finish().map_err(failed)?;
            }
            // ruzstd compresses at its fastest level alone; its others are
            // declared and not implemented, and panic. The declaration says
            // so of `level`.
            Format::Zstd => {
                ruzstd::encoding::compress(
                    input,
                    output,
                    ruzstd::encoding::CompressionLevel::Fastest,
                );
            }
            Format::Zip => {
                zip::write_entry(
                    input,
                    self.entry.as_deref().unwrap_or("content"),
                    level,
                    output,
                )?;
            }
        }
        Ok(())
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
