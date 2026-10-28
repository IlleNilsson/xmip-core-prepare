//! Text normalization: the byte order mark a Stream starts with and the
//! line endings it uses, each removed, added or rewritten as the step
//! says, a piece at a time. The mark is UTF-8's (EF BB BF); a Stream in
//! another encoding is Character Encoding's first, which carries a mark
//! over as U+FEFF, so it arrives here as UTF-8's.

use std::io::{Read, Write};

use crate::declaration::{TEXT_NORMALIZATION as NAME, TEXT_NORMALIZATION_UNIT};
use crate::{CHUNK, Prepare, PrepareError, PrepareUnit, Stage, Values};

const MARK: &[u8] = b"\xEF\xBB\xBF";

/// Text normalization, as the standard module declares it.
pub const UNIT: PrepareUnit = PrepareUnit::new(TEXT_NORMALIZATION_UNIT, open);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mark {
    Keep,
    Remove,
    Add,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ending {
    Keep,
    Lf,
    CrLf,
}

struct TextNormalization {
    mark: Mark,
    ending: Ending,
}

// The shape every unit's open has (`node::unit::Unit`); this one refuses
// nothing its declaration has not refused already.
#[allow(clippy::unnecessary_wraps)]
fn open(_: Stage, values: &Values) -> Result<Box<dyn Prepare>, String> {
    Ok(Box::new(TextNormalization {
        mark: match values.text("byte_order_mark") {
            "remove" => Mark::Remove,
            "add" => Mark::Add,
            _ => Mark::Keep,
        },
        ending: match values.text("line_ending") {
            "lf" => Ending::Lf,
            "crlf" => Ending::CrLf,
            _ => Ending::Keep,
        },
    }))
}

impl Prepare for TextNormalization {
    fn prepare(
        &self,
        input: &mut (dyn Read + Send),
        output: &mut dyn Write,
    ) -> Result<(), PrepareError> {
        let failed = |failed: std::io::Error| PrepareError::of_io(NAME, &failed);
        let mut chunk = vec![0u8; CHUNK];
        let mut held = 0;
        // The first three bytes decide the mark; a short Stream is all of it.
        while held < MARK.len() {
            let read = input.read(&mut chunk[held..]).map_err(failed)?;
            if read == 0 {
                break;
            }
            held += read;
        }
        let marked = chunk[..held].starts_with(MARK);
        let skip = match self.mark {
            Mark::Remove if marked => MARK.len(),
            Mark::Add if !marked => {
                output.write_all(MARK).map_err(failed)?;
                0
            }
            _ => 0,
        };
        let mut lines = Lines {
            ending: self.ending,
            after_cr: false,
            written: Vec::with_capacity(CHUNK + CHUNK / 2),
        };
        let mut piece = &chunk[skip..held];
        loop {
            lines.write(piece, output).map_err(failed)?;
            let read = input.read(&mut chunk).map_err(failed)?;
            if read == 0 {
                break;
            }
            piece = &chunk[..read];
        }
        lines.finish(output).map_err(failed)
    }
}

/// Line endings rewritten across pieces: a CR at the end of one piece is
/// held until the next says whether an LF follows it.
struct Lines {
    ending: Ending,
    after_cr: bool,
    written: Vec<u8>,
}

impl Lines {
    fn write(&mut self, piece: &[u8], output: &mut dyn Write) -> std::io::Result<()> {
        if self.ending == Ending::Keep {
            return output.write_all(piece);
        }
        self.written.clear();
        for &byte in piece {
            match (self.ending, byte, self.after_cr) {
                (Ending::Lf, b'\n', _) => self.written.push(b'\n'),
                (Ending::Lf, b'\r', true) => self.written.push(b'\r'),
                (Ending::Lf, b'\r', false) => {}
                (Ending::Lf, other, true) => self.written.extend_from_slice(&[b'\r', other]),
                (Ending::CrLf, b'\n', false) => self.written.extend_from_slice(b"\r\n"),
                (_, other, _) => self.written.push(other),
            }
            self.after_cr = byte == b'\r';
        }
        output.write_all(&self.written)
    }

    fn finish(&mut self, output: &mut dyn Write) -> std::io::Result<()> {
        if self.ending == Ending::Lf && self.after_cr {
            output.write_all(b"\r")?;
        }
        output.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{prepared, text};

    fn normalized(given: &[(&str, &str)], input: &[u8]) -> Vec<u8> {
        let given: Vec<(&str, xcore::settings::Given)> = given
            .iter()
            .map(|(name, value)| (*name, text(value)))
            .collect();
        prepared(NAME, Stage::Receive, &given, input).expect("normalized")
    }

    #[test]
    fn a_byte_order_mark_is_kept_removed_or_added() {
        let marked = b"\xEF\xBB\xBFa\r\nb";
        assert_eq!(normalized(&[], marked), marked);
        assert_eq!(
            normalized(&[("byte_order_mark", "remove")], marked),
            b"a\r\nb"
        );
        assert_eq!(normalized(&[("byte_order_mark", "remove")], b"ab"), b"ab");
        assert_eq!(
            normalized(&[("byte_order_mark", "add")], b"ab"),
            b"\xEF\xBB\xBFab"
        );
        assert_eq!(normalized(&[("byte_order_mark", "add")], marked), marked);
        assert_eq!(normalized(&[("byte_order_mark", "remove")], b""), b"");
        assert_eq!(
            normalized(&[("byte_order_mark", "remove")], b"\xEF\xBB"),
            b"\xEF\xBB"
        );
    }

    #[test]
    fn line_endings_are_written_as_one_kind() {
        let mixed = b"a\r\nb\nc\rd\r\r\ne\r";
        assert_eq!(
            normalized(&[("line_ending", "lf")], mixed),
            b"a\nb\nc\rd\r\ne\r"
        );
        assert_eq!(
            normalized(&[("line_ending", "crlf")], mixed),
            b"a\r\nb\r\nc\rd\r\r\ne\r"
        );
        assert_eq!(normalized(&[("line_ending", "keep")], mixed), mixed);
    }

    #[test]
    fn a_line_ending_cut_between_two_pieces_is_rewritten_whole() {
        let mut input = vec![b'x'; CHUNK - 1];
        input.extend_from_slice(b"\r\ny");
        let mut expected = vec![b'x'; CHUNK - 1];
        expected.extend_from_slice(b"\ny");
        // The first piece is CHUNK bytes and ends on the CR.
        let given = [("line_ending", text("lf"))];
        assert_eq!(
            prepared(NAME, Stage::Send, &given, &input).expect("normalized"),
            expected
        );
    }
}
