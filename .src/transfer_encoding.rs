//! Transfer encoding: bytes spelled as text so a text-only carrier takes
//! them, Base 64 (RFC 4648) above all — decoded on receive, encoded on
//! send, a piece at a time. The spelling is `xmip-core-library-codec`'s,
//! the estate's one; this cuts a Stream into the pieces it spells.

use std::io::{Read, Write};

use crate::declaration::{TRANSFER_ENCODING as NAME, TRANSFER_ENCODING_UNIT};
use crate::direction::Direction;
use crate::{CHUNK, Prepare, PrepareError, PrepareUnit, Stage, Values};
use xcore::kept::Kept;

/// Transfer encoding, as the standard module declares it.
pub const UNIT: PrepareUnit = PrepareUnit::new(TRANSFER_ENCODING_UNIT, open);

struct TransferEncoding {
    url: bool,
    line: usize,
    direction: Direction,
    kept: Kept<Pieces>,
}

/// The piece read and the text spelled, kept between Streams.
struct Pieces {
    read: Vec<u8>,
    text: String,
}

impl Pieces {
    fn new() -> Self {
        Self {
            read: vec![0u8; CHUNK],
            text: String::with_capacity(CHUNK + CHUNK / 2),
        }
    }
}

fn open(stage: Stage, values: &Values) -> Result<Box<dyn Prepare>, String> {
    Ok(Box::new(TransferEncoding {
        url: values.text("format") == "base64url",
        line: usize::try_from(values.integer("line_length")).unwrap_or(0),
        direction: Direction::of(NAME, stage, values, ["decode", "encode"])?,
        kept: Kept::new(Pieces::new),
    }))
}

fn refused(why: impl std::fmt::Display) -> PrepareError {
    PrepareError::new(format!("{NAME}: {why}"))
}

impl Prepare for TransferEncoding {
    fn prepare(
        &self,
        input: &mut (dyn Read + Send),
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        self.kept.with(|pieces| match self.direction {
            Direction::Inward => self.decode(pieces, input, output),
            Direction::Outward => self.encode(pieces, input, output),
        })
    }
}

impl TransferEncoding {
    /// Spell the bytes three at a time, so every piece but the last ends
    /// on a whole group and only the last can carry padding.
    fn encode(
        &self,
        pieces: &mut Pieces,
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        let failed = |failed: std::io::Error| PrepareError::of_io(NAME, &failed);
        let chunk = &mut pieces.read[..CHUNK / 4 * 3];
        let lines = &mut pieces.text;
        let mut column = 0;
        loop {
            let held = fill(input, chunk).map_err(failed)?;
            if held == 0 {
                break;
            }
            let spelled = if self.url {
                codec::base64::encode_url_unpadded(&chunk[..held])
            } else {
                codec::base64::encode(&chunk[..held])
            };
            if self.line == 0 {
                output.write_all(spelled.as_bytes()).map_err(failed)?;
            } else {
                lines.clear();
                for character in spelled.chars() {
                    if column == self.line {
                        lines.push_str("\r\n");
                        column = 0;
                    }
                    lines.push(character);
                    column += 1;
                }
                output.write_all(lines.as_bytes()).map_err(failed)?;
            }
            if held < chunk.len() {
                break;
            }
        }
        output.flush().map_err(failed)
    }

    /// Read the text four characters at a time, line breaks stepped over
    /// as MIME wraps it; anything after the padding is refused.
    fn decode(
        &self,
        pieces: &mut Pieces,
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        let failed = |failed: std::io::Error| PrepareError::of_io(NAME, &failed);
        let Pieces {
            read: chunk,
            text: digits,
        } = pieces;
        digits.clear();
        let mut padded = false;
        let mut read_so_far = 0u64;
        loop {
            let read = input.read(chunk).map_err(failed)?;
            for &byte in &chunk[..read] {
                if byte == b'\r' || byte == b'\n' {
                    continue;
                }
                if padded && byte != b'=' {
                    return Err(refused(format!(
                        "text follows the padding near byte {read_so_far}"
                    )));
                }
                padded |= byte == b'=';
                digits.push(char::from(byte));
            }
            read_so_far += read as u64;
            let whole = if read == 0 {
                digits.len()
            } else {
                digits.len() / 4 * 4
            };
            if whole > 0 && !(padded && read != 0) {
                let bytes = if self.url {
                    codec::base64::decode_url(&digits[..whole])
                } else {
                    codec::base64::decode(&digits[..whole])
                }
                .map_err(|refusal| refused(format!("{refusal} near byte {read_so_far}")))?;
                output.write_all(&bytes).map_err(failed)?;
                digits.drain(..whole);
            }
            if read == 0 {
                break;
            }
        }
        output.flush().map_err(failed)
    }
}

/// Fill `into` from `input` as far as it goes: short only at the end.
fn fill(input: &mut dyn Read, into: &mut [u8]) -> std::io::Result<usize> {
    let mut held = 0;
    while held < into.len() {
        match input.read(&mut into[held..])? {
            0 => break,
            read => held += read,
        }
    }
    Ok(held)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{prepared, text};
    use xcore::settings::Given;

    #[test]
    fn bytes_are_encoded_on_send_and_decoded_on_receive() {
        let bytes: Vec<u8> = (0..=255).collect();
        let encoded = prepared(NAME, Stage::Send, &[], &bytes).expect("encoded");
        assert_eq!(encoded, codec::base64::encode(&bytes).as_bytes());
        assert_eq!(
            prepared(NAME, Stage::Receive, &[], &encoded).expect("decoded"),
            bytes
        );
    }

    #[test]
    fn lines_are_wrapped_and_read_back_over_their_breaks() {
        let bytes = vec![0xA5u8; 200];
        let given = [("line_length", Given::Integer(76))];
        let encoded = prepared(NAME, Stage::Send, &given, &bytes).expect("encoded");
        let text_lines: Vec<&[u8]> = encoded.split(|byte| *byte == b'\n').collect();
        assert!(
            text_lines.iter().all(|line| line.len() <= 77),
            "{text_lines:?}"
        );
        assert_eq!(
            prepared(NAME, Stage::Receive, &given, &encoded).expect("decoded"),
            bytes
        );
    }

    #[test]
    fn the_url_alphabet_is_written_without_padding() {
        let given = [("format", text("base64url"))];
        let encoded = prepared(NAME, Stage::Send, &given, b"\xFB\xFF").expect("encoded");
        assert_eq!(encoded, b"-_8");
        assert_eq!(
            prepared(NAME, Stage::Receive, &given, b"-_8").expect("decoded"),
            b"\xFB\xFF"
        );
    }

    #[test]
    fn text_after_the_padding_and_a_foreign_character_are_refused() {
        let refused = prepared(NAME, Stage::Receive, &[], b"QQ==QQ==").expect_err("after");
        assert!(
            refused.to_string().contains("follows the padding"),
            "{refused}"
        );
        let refused = prepared(NAME, Stage::Receive, &[], b"QQ*=").expect_err("foreign");
        assert!(
            refused.to_string().starts_with("TransferEncoding:"),
            "{refused}"
        );
    }

    #[test]
    fn a_stream_longer_than_a_piece_is_spelled_as_one() {
        let bytes: Vec<u8> = (0..CHUNK * 3 + 7)
            .map(|at| u8::try_from(at % 253).expect("below 253"))
            .collect();
        let encoded = prepared(NAME, Stage::Send, &[], &bytes).expect("encoded");
        assert_eq!(encoded, codec::base64::encode(&bytes).as_bytes());
        assert_eq!(
            prepared(NAME, Stage::Receive, &[], &encoded).expect("decoded"),
            bytes
        );
    }
}
