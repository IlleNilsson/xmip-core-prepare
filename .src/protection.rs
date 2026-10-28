//! Encryption and Signature: a Stream decrypted and its signature verified
//! on receive, encrypted and signed on send, in the format a step names —
//! `OpenPGP`, CMS, JOSE — through the engine that implements it. The engines
//! are technologies of this capability behind [`ProtectionEngine`], handed
//! in by the module that links them, the first `rpgp` for `OpenPGP`; a step
//! chooses one by `engine`, as it chooses an XSLT engine.
//!
//! Keys are files the step names, read when it is opened. Nothing is
//! looked up over a network (ADR-0045, amendment 2026-10-22): no key
//! server, no online revocation check, no certificate chased by its
//! address; an engine that can is not admitted.

use std::path::PathBuf;

use crate::declaration::{ENCRYPTION, SIGNATURE};
use crate::direction::Direction;
use crate::{Prepare, Stage, Values};

/// The keys a step names, read when it is opened.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// This end's own key: what decrypts on receive and signs on send.
    pub own: Option<PathBuf>,
    /// The far ends' public keys or certificates: what encrypts on send and
    /// verifies on receive.
    pub theirs: Vec<PathBuf>,
}

/// An engine of one format: a technology of this capability.
pub trait ProtectionEngine: Send + Sync {
    /// Its name, as a step's `engine` names it.
    fn name(&self) -> &'static str;

    /// The format it implements: `openpgp`, `cms` or `jose`.
    fn format(&self) -> &'static str;

    /// Encryption opened: decrypting `Inward`, encrypting `Outward`.
    ///
    /// # Errors
    /// A key cannot be read or is not one the format takes, in words.
    fn encryption(&self, direction: Direction, keys: &Keys) -> Result<Box<dyn Prepare>, String>;

    /// Signature opened: verifying `Inward`, signing `Outward`.
    ///
    /// # Errors
    /// As [`ProtectionEngine::encryption`].
    fn signature(&self, direction: Direction, keys: &Keys) -> Result<Box<dyn Prepare>, String>;
}

/// The engine a step names, of the format it names.
fn engine<'a>(
    unit: &str,
    engines: &[&'a dyn ProtectionEngine],
    values: &Values,
) -> Result<&'a dyn ProtectionEngine, String> {
    let (format, named) = (values.text("format"), values.text("engine"));
    engines
        .iter()
        .copied()
        .find(|engine| engine.format() == format && engine.name() == named)
        .ok_or_else(|| {
            let linked: Vec<String> = engines
                .iter()
                .filter(|engine| engine.format() == format)
                .map(|engine| engine.name().to_string())
                .collect();
            format!(
                "{unit}: no {format} engine called '{named}' is in this node; it has {}",
                if linked.is_empty() {
                    "none".to_string()
                } else {
                    linked.join(", ")
                }
            )
        })
}

fn files(text: Option<&str>) -> Vec<PathBuf> {
    text.map(|text| {
        text.split(',')
            .map(str::trim)
            .filter(|file| !file.is_empty())
            .map(PathBuf::from)
            .collect()
    })
    .unwrap_or_default()
}

/// Encryption opened for a step at `stage` with `values`, through one of
/// `engines`.
///
/// # Errors
/// No engine of the format is called as the step names, the step names no
/// key the direction needs, or the engine refuses a key.
pub fn open_encryption(
    engines: &[&dyn ProtectionEngine],
    stage: Stage,
    values: &Values,
) -> Result<Box<dyn Prepare>, String> {
    let direction = Direction::of(ENCRYPTION, stage, values, ["decrypt", "encrypt"])?;
    let keys = Keys {
        own: values.optional_text("key").map(PathBuf::from),
        theirs: files(values.optional_text("recipients")),
    };
    match direction {
        Direction::Inward if keys.own.is_none() => {
            return Err(format!("{ENCRYPTION}: decrypting needs this end's `key`"));
        }
        Direction::Outward if keys.theirs.is_empty() => {
            return Err(format!("{ENCRYPTION}: encrypting needs the `recipients`"));
        }
        _ => {}
    }
    engine(ENCRYPTION, engines, values)?.encryption(direction, &keys)
}

/// Signature opened for a step at `stage` with `values`, through one of
/// `engines`.
///
/// # Errors
/// As [`open_encryption`].
pub fn open_signature(
    engines: &[&dyn ProtectionEngine],
    stage: Stage,
    values: &Values,
) -> Result<Box<dyn Prepare>, String> {
    let direction = Direction::of(SIGNATURE, stage, values, ["verify", "sign"])?;
    let keys = Keys {
        own: values.optional_text("key").map(PathBuf::from),
        theirs: files(values.optional_text("signers")),
    };
    match direction {
        Direction::Inward if keys.theirs.is_empty() => {
            return Err(format!("{SIGNATURE}: verifying needs the `signers`"));
        }
        Direction::Outward if keys.own.is_none() => {
            return Err(format!("{SIGNATURE}: signing needs this end's `key`"));
        }
        _ => {}
    }
    engine(SIGNATURE, engines, values)?.signature(direction, &keys)
}
