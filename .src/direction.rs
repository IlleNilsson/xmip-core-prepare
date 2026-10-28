//! Which way a prepare unit works: undoing what the far end did, or doing
//! it for the far end. One rule for every unit, so Compression, Character
//! Encoding and the rest decide it alike.

use crate::{Stage, Values};

/// Which way a unit works on the Stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Undo what the far end did: decompress, decode, decrypt, verify.
    /// What Receive Preparation does unless its step says otherwise.
    Inward,
    /// Do it for the far end: compress, encode, encrypt, sign. What Send
    /// Preparation does unless its step says otherwise.
    Outward,
}

pub use crate::declaration::DIRECTION as SETTING;

impl Direction {
    /// The direction a unit called `unit` works in at `stage`: the
    /// step's `direction` where it gives one, as `words` spell the two
    /// ways (inward first), else inward on receive and outward on send.
    /// Process Preparation has no way of its own, so a step there names
    /// one.
    ///
    /// # Errors
    /// A step at Process Preparation that names no direction, in words.
    pub fn of(unit: &str, stage: Stage, values: &Values, words: [&str; 2]) -> Result<Self, String> {
        match (values.optional_text(SETTING), stage) {
            (Some(word), _) if word == words[0] => Ok(Self::Inward),
            (Some(_), _) | (None, Stage::Send) => Ok(Self::Outward),
            (None, Stage::Receive) => Ok(Self::Inward),
            (None, Stage::Process) => Err(format!(
                "the unit '{unit}' at Process Preparation needs its direction: \
                 '{}' or '{}'",
                words[0], words[1]
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xcore::settings::{Applies, Given, Kind, Presence, Setting, Settings};

    const DECLARED: &Settings = &Settings {
        technology: "Example",
        settings: &[Setting {
            name: SETTING,
            kind: Kind::Choice {
                choices: &["undo", "do"],
            },
            presence: Presence::Optional,
            meaning: "Which way the unit works.",
            applies: Applies::Both,
        }],
    };

    fn values(given: &[(&str, &str)]) -> Values {
        let given: Vec<(String, Given)> = given
            .iter()
            .map(|(name, value)| ((*name).to_string(), Given::Text((*value).to_string())))
            .collect();
        DECLARED.read(Applies::Both, &given).expect("reads")
    }

    #[test]
    fn receive_undoes_send_does_and_a_step_may_say_otherwise() {
        let none = values(&[]);
        let of = |stage, values: &Values| Direction::of("Example", stage, values, ["undo", "do"]);
        assert_eq!(of(Stage::Receive, &none).expect("ok"), Direction::Inward);
        assert_eq!(of(Stage::Send, &none).expect("ok"), Direction::Outward);
        let refused = of(Stage::Process, &none).expect_err("needs one");
        assert!(refused.contains("'undo' or 'do'"), "{refused}");
        let doing = values(&[(SETTING, "do")]);
        assert_eq!(of(Stage::Receive, &doing).expect("ok"), Direction::Outward);
        assert_eq!(
            of(Stage::Process, &values(&[(SETTING, "undo")])).expect("ok"),
            Direction::Inward
        );
    }
}
