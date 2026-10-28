#![forbid(unsafe_code)]

//! Preparation: the standard prepare units, each working on a Stream's
//! bytes and never on a Message (`runtime-model.md` section 8) — before a
//! Message exists on receive, after serialization on send.
//!
//! A unit is one function for both ways (the owner, 2026-10-22: *The code
//! used in a prepare situation is usually similar. It is just an enum away
//! what it does in what situation*): it is opened for the stage its step
//! stands at — Receive Preparation, Process Preparation or Send
//! Preparation — and undoes on receive what the far end did (decompress,
//! decode, decrypt, verify) and does it on send. At Process Preparation, or
//! where a step says so, its `direction` setting decides ([`Direction`]).
//!
//! Every unit streams: it reads its input a piece at a time and writes its
//! output as it goes, so no unit holds the whole Stream, however large.
//!
//! What each unit is, its [`declaration`], is compiled always; the units'
//! code and its dependencies come with the default feature `units`, so the
//! configuration knows the standard units without linking them.

pub mod declaration;

#[cfg(feature = "units")]
pub mod character_encoding;
#[cfg(feature = "units")]
pub mod compression;
#[cfg(feature = "units")]
mod direction;
#[cfg(feature = "units")]
pub mod protection;
#[cfg(feature = "units")]
pub mod text_normalization;
#[cfg(feature = "units")]
pub mod transfer_encoding;
#[cfg(feature = "units")]
mod zip;

pub use declaration::{DECLARATIONS, EVERY_STAGE};
#[cfg(feature = "units")]
pub use direction::Direction;
pub use node::Stage;

#[cfg(feature = "units")]
pub use units::*;

#[cfg(feature = "units")]
mod units {
    use std::io::{Read, Write};

    use xcore::settings::Given;

    use crate::Stage;

    /// What a step gave a unit, read through its declaration.
    pub type Values = xcore::settings::Read;

    xcore::declare_error!(PrepareError);

    impl PrepareError {
        /// What reading or writing the Stream refused, named by the unit.
        #[must_use]
        pub fn of_io(unit: &str, failed: &std::io::Error) -> Self {
            Self::new(format!("{unit}: {failed}"))
        }
    }

    /// One prepare unit, opened for one step at one stage.
    pub trait Prepare: Send + Sync {
        /// Prepare `input` into `output`, a piece at a time.
        ///
        /// # Errors
        /// The Stream is not what the unit undoes — not compressed as named,
        /// not text in the named encoding — or could not be read or written.
        fn prepare(
            &self,
            input: &mut (dyn Read + Send),
            output: &mut dyn Write,
        ) -> Result<(), PrepareError>;
    }

    /// A standard prepare unit, linked: its declaration and how it opens.
    pub type PrepareUnit = node::unit::Unit<dyn Prepare>;

    /// The standard prepare units this crate opens by itself, in the order
    /// a person reads them. Encryption and Signature are opened over the
    /// engines the module linking them hands in ([`crate::protection`]).
    pub static UNITS: [&PrepareUnit; 4] = [
        &crate::compression::UNIT,
        &crate::character_encoding::UNIT,
        &crate::transfer_encoding::UNIT,
        &crate::text_normalization::UNIT,
    ];

    /// The standard prepare unit called `name`, of those this crate opens.
    #[must_use]
    pub fn unit(name: &str) -> Option<&'static PrepareUnit> {
        UNITS
            .iter()
            .copied()
            .find(|unit| unit.declaration.name == name)
    }

    /// Open the standard prepare unit called `name` for a step at `stage`
    /// that gave `given`.
    ///
    /// # Errors
    /// No standard prepare unit this crate opens is called `name`, or it
    /// refuses the stage or the settings.
    pub fn open(
        name: &str,
        stage: Stage,
        given: &[(String, Given)],
    ) -> Result<Box<dyn Prepare>, PrepareError> {
        unit(name)
            .ok_or_else(|| {
                PrepareError::new(format!("no standard prepare unit is called '{name}'"))
            })?
            .open(stage, given)
            .map_err(PrepareError::new)
    }

    /// Copy `input` to `output`: the one copy every unit's drain goes
    /// through.
    ///
    /// # Errors
    /// Reading or writing failed; the error names `unit`.
    pub(crate) fn copy(
        unit: &str,
        input: &mut dyn Read,
        output: &mut dyn Write,
    ) -> Result<u64, PrepareError> {
        std::io::copy(input, output).map_err(|failed| PrepareError::of_io(unit, &failed))
    }

    /// How much of a Stream a unit holds at a time where it holds a piece.
    pub(crate) const CHUNK: usize = 64 * 1024;

    /// `input` prepared by the unit called `name` at `stage` with `given`.
    #[cfg(test)]
    pub(crate) fn prepared(
        name: &str,
        stage: Stage,
        given: &[(&str, Given)],
        input: &[u8],
    ) -> Result<Vec<u8>, PrepareError> {
        let given: Vec<(String, Given)> = given
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect();
        let mut output = Vec::new();
        open(name, stage, &given)?.prepare(&mut &input[..], &mut output)?;
        Ok(output)
    }

    /// A text setting, as a step gives it.
    #[cfg(test)]
    pub(crate) fn text(value: &str) -> Given {
        Given::Text(value.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn every_unit_opens_from_a_declaration_this_crate_gives() {
            for unit in UNITS {
                assert!(crate::DECLARATIONS.contains(&unit.declaration));
                assert!(std::ptr::eq(
                    super::unit(unit.declaration.name).expect("found"),
                    unit
                ));
            }
            assert!(super::unit("Nothing").is_none());
        }

        #[test]
        fn a_unit_refuses_a_setting_it_does_not_declare_and_a_name_none_has() {
            let refused = open(
                crate::declaration::COMPRESSION,
                Stage::Receive,
                &[("speed".into(), Given::Integer(1))],
            )
            .err()
            .expect("refused");
            assert!(refused.to_string().contains("speed"), "{refused}");
            let refused = open("Nothing", Stage::Receive, &[]).err().expect("refused");
            assert!(refused.to_string().contains("'Nothing'"), "{refused}");
        }
    }
}
