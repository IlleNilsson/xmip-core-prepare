//! Character encoding: text in the far end's encoding read into UTF-8 on
//! receive, and UTF-8 written in the far end's on send, a piece at a time.
//! Strict both ways (ADR-0038, amendment 2026-09-26): bytes that are not
//! the named encoding, and a character the named encoding cannot write,
//! are refused, never replaced, because a replacement character is a
//! payload silently changed.
//!
//! The encodings are the WHATWG Encoding Standard's, by any of its labels
//! (`windows-1252`, `iso-8859-1`, `shift_jis`, …), through `encoding_rs`;
//! the Unicode forms that standard does not write — UTF-16 out, and UTF-32
//! both ways — are `xmip-core-library-codec`'s (`codec::unicode`). A byte
//! order mark is carried over as U+FEFF, for Text Normalization to keep or
//! remove.

use std::io::{Read, Write};

use codec::unicode::Form;
use encoding_rs::{DecoderResult, EncoderResult, Encoding};

use crate::declaration::{CHARACTER_ENCODING as NAME, CHARACTER_ENCODING_UNIT};
use crate::direction::Direction;
use crate::{CHUNK, Prepare, PrepareError, PrepareUnit, Stage, Values};

/// Character encoding, as the standard module declares it.
pub const UNIT: PrepareUnit = PrepareUnit::new(CHARACTER_ENCODING_UNIT, open);

/// Where an encoding is written and read.
#[derive(Clone, Copy, Debug)]
enum Named {
    /// The WHATWG standard's, through `encoding_rs`.
    Standard(&'static Encoding),
    /// A Unicode form, through `codec::unicode`, where the standard does not
    /// write it.
    Form(Form),
}

struct CharacterEncoding {
    named: Named,
    direction: Direction,
}

fn open(stage: Stage, values: &Values) -> Result<Box<dyn Prepare>, String> {
    let label = values.text("encoding");
    let direction = Direction::of(NAME, stage, values, ["decode", "encode"])?;
    let form = Form::named(&label.to_ascii_lowercase()).ok();
    let named = match (Encoding::for_label(label.as_bytes()), form) {
        // UTF-32 is no WHATWG encoding, and UTF-16 is one the standard
        // reads and never writes.
        (_, Some(form @ (Form::Utf32Le | Form::Utf32Be))) => Named::Form(form),
        (_, Some(form @ (Form::Utf16Le | Form::Utf16Be))) if direction == Direction::Outward => {
            Named::Form(form)
        }
        (Some(encoding), _)
            if encoding.is_single_byte() || encoding.output_encoding() == encoding =>
        {
            Named::Standard(encoding)
        }
        (Some(encoding), _) if direction == Direction::Inward => Named::Standard(encoding),
        (Some(encoding), _) => {
            return Err(format!(
                "{NAME}: '{}' is read, never written",
                encoding.name()
            ));
        }
        (None, _) => {
            return Err(format!("{NAME}: '{label}' names no encoding"));
        }
    };
    Ok(Box::new(CharacterEncoding { named, direction }))
}

fn refused(why: impl std::fmt::Display) -> PrepareError {
    PrepareError::new(format!("{NAME}: {why}"))
}

impl Prepare for CharacterEncoding {
    fn prepare(
        &self,
        input: &mut (dyn Read + Send),
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        match (self.named, self.direction) {
            (Named::Standard(encoding), Direction::Inward) => decode(encoding, input, output),
            (Named::Standard(encoding), Direction::Outward) => encode(encoding, input, output),
            (Named::Form(form), Direction::Inward) => decode_form(form, input, output),
            (Named::Form(form), Direction::Outward) => encode_form(form, input, output),
        }
    }
}

fn io(failed: &std::io::Error) -> PrepareError {
    PrepareError::of_io(NAME, failed)
}

fn decode(
    encoding: &'static Encoding,
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), PrepareError> {
    let mut decoder = encoding.new_decoder_without_bom_handling();
    let mut chunk = vec![0u8; CHUNK];
    let mut text = vec![0u8; CHUNK * 3 + 16];
    let mut offset = 0u64;
    loop {
        let read = input.read(&mut chunk).map_err(|failed| io(&failed))?;
        let last = read == 0;
        let mut rest = &chunk[..read];
        loop {
            let (result, consumed, written) =
                decoder.decode_to_utf8_without_replacement(rest, &mut text, last);
            output
                .write_all(&text[..written])
                .map_err(|failed| io(&failed))?;
            offset += consumed as u64;
            rest = &rest[consumed..];
            match result {
                DecoderResult::InputEmpty => break,
                DecoderResult::OutputFull => {}
                DecoderResult::Malformed(length, _) => {
                    return Err(refused(format!(
                        "the bytes are not {} near byte {}",
                        encoding.name(),
                        offset.saturating_sub(u64::from(length))
                    )));
                }
            }
        }
        if last {
            return output.flush().map_err(|failed| io(&failed));
        }
    }
}

fn encode(
    encoding: &'static Encoding,
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), PrepareError> {
    let mut encoder = encoding.new_encoder();
    let mut pieces = Utf8Pieces::new();
    let mut bytes = vec![0u8; CHUNK * 2 + 16];
    while let Some((mut rest, last)) = pieces.next(input)? {
        loop {
            let (result, consumed, written) =
                encoder.encode_from_utf8_without_replacement(rest, &mut bytes, last);
            output
                .write_all(&bytes[..written])
                .map_err(|failed| io(&failed))?;
            rest = &rest[consumed..];
            match result {
                EncoderResult::InputEmpty => break,
                EncoderResult::OutputFull => {}
                EncoderResult::Unmappable(character) => {
                    return Err(refused(format!(
                        "{} cannot write U+{:04X}",
                        encoding.name(),
                        u32::from(character)
                    )));
                }
            }
        }
        if last {
            break;
        }
    }
    output.flush().map_err(|failed| io(&failed))
}

fn decode_form(
    form: Form,
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), PrepareError> {
    // A UTF-32 code unit is four bytes and stands alone, so a piece cut on
    // a multiple of four reads by itself.
    let mut chunk = vec![0u8; CHUNK];
    let mut held = 0;
    loop {
        let read = input
            .read(&mut chunk[held..])
            .map_err(|failed| io(&failed))?;
        held += read;
        let whole = if read == 0 { held } else { held / 4 * 4 };
        let text = form.decode(&chunk[..whole]).map_err(refused)?;
        output
            .write_all(text.as_bytes())
            .map_err(|failed| io(&failed))?;
        chunk.copy_within(whole..held, 0);
        held -= whole;
        if read == 0 {
            return output.flush().map_err(|failed| io(&failed));
        }
    }
}

fn encode_form(
    form: Form,
    input: &mut dyn Read,
    output: &mut dyn Write,
) -> Result<(), PrepareError> {
    let mut pieces = Utf8Pieces::new();
    while let Some((piece, last)) = pieces.next(input)? {
        output
            .write_all(&form.encode(piece))
            .map_err(|failed| io(&failed))?;
        if last {
            break;
        }
    }
    output.flush().map_err(|failed| io(&failed))
}

/// UTF-8 read a piece at a time, each piece cut on a whole character, so
/// a character never straddles two.
struct Utf8Pieces {
    chunk: Vec<u8>,
    held: usize,
    offset: u64,
    text: String,
}

impl Utf8Pieces {
    fn new() -> Self {
        Self {
            chunk: vec![0u8; CHUNK],
            held: 0,
            offset: 0,
            text: String::new(),
        }
    }

    /// The next piece, and whether it is the last.
    fn next(&mut self, input: &mut dyn Read) -> Result<Option<(&str, bool)>, PrepareError> {
        let read = input
            .read(&mut self.chunk[self.held..])
            .map_err(|failed| io(&failed))?;
        let total = self.held + read;
        let last = read == 0;
        let (whole, valid) = match std::str::from_utf8(&self.chunk[..total]) {
            Ok(_) => (total, true),
            // An incomplete character at the end waits for the next piece.
            Err(error) if error.error_len().is_none() && !last => (error.valid_up_to(), true),
            Err(error) => (error.valid_up_to(), false),
        };
        if !valid {
            return Err(refused(format!(
                "the text is not UTF-8 near byte {}",
                self.offset + whole as u64
            )));
        }
        self.text.clear();
        self.text
            .push_str(std::str::from_utf8(&self.chunk[..whole]).unwrap_or_default());
        self.chunk.copy_within(whole..total, 0);
        self.held = total - whole;
        self.offset += whole as u64;
        Ok(Some((&self.text, last)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{prepared, text};

    fn at(stage: Stage, encoding: &str, input: &[u8]) -> Result<Vec<u8>, PrepareError> {
        prepared(NAME, stage, &[("encoding", text(encoding))], input)
    }

    #[test]
    fn a_legacy_encoding_is_read_into_utf8_and_written_back() {
        let latin = b"caf\xE9 \x80";
        let utf8 = at(Stage::Receive, "windows-1252", latin).expect("read");
        assert_eq!(utf8, "café €".as_bytes());
        assert_eq!(
            at(Stage::Send, "windows-1252", &utf8).expect("written"),
            latin
        );
        let japanese = at(Stage::Send, "shift_jis", "日本".as_bytes()).expect("written");
        assert_eq!(
            at(Stage::Receive, "shift_jis", &japanese).expect("read"),
            "日本".as_bytes()
        );
    }

    #[test]
    fn the_unicode_forms_go_both_ways() {
        for form in ["utf-16le", "utf-16be", "utf-32le", "utf-32be"] {
            let written = at(Stage::Send, form, "a€😀".as_bytes()).expect("written");
            assert_eq!(
                at(Stage::Receive, form, &written).expect("read"),
                "a€😀".as_bytes()
            );
        }
    }

    #[test]
    fn what_is_not_the_encoding_or_cannot_be_written_is_refused_never_replaced() {
        let refused = at(Stage::Receive, "utf-8", b"ok\xFF").expect_err("not utf-8");
        assert!(refused.to_string().contains("near byte 2"), "{refused}");
        let refused = at(Stage::Send, "iso-8859-1", "日".as_bytes()).expect_err("unmappable");
        assert!(refused.to_string().contains("U+65E5"), "{refused}");
        let refused = at(Stage::Send, "utf-8", b"\xC3").expect_err("cut");
        assert!(refused.to_string().contains("not UTF-8"), "{refused}");
        let refused = at(Stage::Receive, "klingon", b"").expect_err("no encoding");
        assert!(
            refused.to_string().contains("'klingon' names no encoding"),
            "{refused}"
        );
    }

    #[test]
    fn a_character_cut_between_two_pieces_is_read_whole() {
        let mut text_in = "x".repeat(CHUNK - 1);
        text_in.push('€');
        let written = at(Stage::Send, "utf-16be", text_in.as_bytes()).expect("written");
        assert_eq!(
            at(Stage::Receive, "utf-16be", &written).expect("read"),
            text_in.as_bytes()
        );
        let latin = at(Stage::Send, "windows-1252", text_in.as_bytes()).expect("written");
        assert_eq!(latin.len(), CHUNK);
    }
}
