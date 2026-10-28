//! What each standard prepare unit is, as data: its name, the stages it
//! runs at and what a step gives it (`node::unit::Declaration`). Compiled
//! with or without the units' code, so `configure` knows every standard
//! unit without linking a compressor (the owner, 2026-10-22: the standard
//! module is built in).

use node::Stage;
use node::unit::Declaration;
use xcore::settings::{Applies, Fixed, Kind, Presence, Setting, Settings};

/// Every stage, which every standard prepare unit runs at.
pub const EVERY_STAGE: &[Stage] = &[Stage::Receive, Stage::Process, Stage::Send];

/// The setting a step fixes a unit's direction with, where its stage does
/// not decide it.
pub const DIRECTION: &str = "direction";

const fn direction(ways: &'static [&'static str]) -> Setting {
    Setting {
        name: DIRECTION,
        kind: Kind::Choice { choices: ways },
        presence: Presence::Optional,
        meaning: "Which way it works, where its stage does not decide.",
        applies: Applies::Both,
    }
}

/// Compression's name, as a step names it.
pub const COMPRESSION: &str = "Compression";
/// The compression formats.
pub const COMPRESSION_FORMATS: &[&str] = &["gzip", "zlib", "deflate", "zstd", "zip"];
/// How hard compressing works.
pub const LEVELS: &[&str] = &["fastest", "default", "best"];

/// Compression: decompresses on receive, compresses on send.
pub const COMPRESSION_UNIT: &Declaration = &Declaration {
    name: COMPRESSION,
    capability: "prepare",
    stages: EVERY_STAGE,
    settings: &Settings {
        technology: COMPRESSION,
        settings: &[
            Setting {
                name: "format",
                kind: Kind::Choice {
                    choices: COMPRESSION_FORMATS,
                },
                presence: Presence::Default(Fixed::Text("gzip")),
                meaning: "How the Stream is compressed: gzip, zlib, raw deflate, zstd or a zip archive.",
                applies: Applies::Both,
            },
            Setting {
                name: "level",
                kind: Kind::Choice { choices: LEVELS },
                presence: Presence::Default(Fixed::Text("default")),
                meaning: "How hard compressing works, trading speed for size; zstd has its fastest alone.",
                applies: Applies::Both,
            },
            Setting {
                name: "entry",
                kind: Kind::Text,
                presence: Presence::Optional,
                meaning: "The zip entry read, the first where none is named, or the name written.",
                applies: Applies::Both,
            },
            direction(&["decompress", "compress"]),
        ],
    },
};

/// Character encoding's name, as a step names it.
pub const CHARACTER_ENCODING: &str = "CharacterEncoding";

/// Character encoding: reads the far end's encoding into UTF-8 on receive,
/// writes it on send.
pub const CHARACTER_ENCODING_UNIT: &Declaration = &Declaration {
    name: CHARACTER_ENCODING,
    capability: "prepare",
    stages: EVERY_STAGE,
    settings: &Settings {
        technology: CHARACTER_ENCODING,
        settings: &[
            Setting {
                name: "encoding",
                kind: Kind::Text,
                presence: Presence::Required,
                meaning: "The far end's encoding, by a WHATWG label or utf-16le, utf-16be, utf-32le or utf-32be.",
                applies: Applies::Both,
            },
            direction(&["decode", "encode"]),
        ],
    },
};

/// Transfer encoding's name, as a step names it.
pub const TRANSFER_ENCODING: &str = "TransferEncoding";

/// Transfer encoding: decodes Base 64 on receive, encodes it on send.
pub const TRANSFER_ENCODING_UNIT: &Declaration = &Declaration {
    name: TRANSFER_ENCODING,
    capability: "prepare",
    stages: EVERY_STAGE,
    settings: &Settings {
        technology: TRANSFER_ENCODING,
        settings: &[
            Setting {
                name: "format",
                kind: Kind::Choice {
                    choices: &["base64", "base64url"],
                },
                presence: Presence::Default(Fixed::Text("base64")),
                meaning: "How bytes are spelled: Base 64, or its URL alphabet without padding.",
                applies: Applies::Both,
            },
            Setting {
                name: "line_length",
                kind: Kind::Integer {
                    minimum: 0,
                    maximum: 1_000_000,
                },
                presence: Presence::Default(Fixed::Integer(0)),
                meaning: "The characters an encoded line holds before a CR LF, none where 0.",
                applies: Applies::Both,
            },
            direction(&["decode", "encode"]),
        ],
    },
};

/// Text normalization's name, as a step names it.
pub const TEXT_NORMALIZATION: &str = "TextNormalization";

/// Text normalization: the byte order mark and the line endings.
pub const TEXT_NORMALIZATION_UNIT: &Declaration = &Declaration {
    name: TEXT_NORMALIZATION,
    capability: "prepare",
    stages: EVERY_STAGE,
    settings: &Settings {
        technology: TEXT_NORMALIZATION,
        settings: &[
            Setting {
                name: "byte_order_mark",
                kind: Kind::Choice {
                    choices: &["keep", "remove", "add"],
                },
                presence: Presence::Default(Fixed::Text("keep")),
                meaning: "Whether a leading byte order mark is kept, removed or added where absent.",
                applies: Applies::Both,
            },
            Setting {
                name: "line_ending",
                kind: Kind::Choice {
                    choices: &["keep", "lf", "crlf"],
                },
                presence: Presence::Default(Fixed::Text("keep")),
                meaning: "Whether line endings are kept or all written as LF or as CR LF.",
                applies: Applies::Both,
            },
        ],
    },
};

/// Encryption's name, as a step names it.
pub const ENCRYPTION: &str = "Encryption";
/// Signature's name, as a step names it.
pub const SIGNATURE: &str = "Signature";
/// The formats a Stream is protected in.
pub const PROTECTION_FORMATS: &[&str] = &["openpgp", "cms", "jose"];

const FORMAT: Setting = Setting {
    name: "format",
    kind: Kind::Choice {
        choices: PROTECTION_FORMATS,
    },
    presence: Presence::Required,
    meaning: "The format the Stream is protected in.",
    applies: Applies::Both,
};

const ENGINE: Setting = Setting {
    name: "engine",
    kind: Kind::Text,
    presence: Presence::Required,
    meaning: "The engine of that format the node was built with.",
    applies: Applies::Both,
};

/// Encryption: decrypts on receive, encrypts on send.
pub const ENCRYPTION_UNIT: &Declaration = &Declaration {
    name: ENCRYPTION,
    capability: "prepare",
    stages: EVERY_STAGE,
    settings: &Settings {
        technology: ENCRYPTION,
        settings: &[
            FORMAT,
            ENGINE,
            Setting {
                name: "key",
                kind: Kind::Address,
                presence: Presence::Optional,
                meaning: "This end's private key, which decrypts on receive.",
                applies: Applies::Both,
            },
            Setting {
                name: "recipients",
                kind: Kind::Text,
                presence: Presence::Optional,
                meaning: "The far ends' public keys or certificates, files separated by commas, which encrypt on send.",
                applies: Applies::Both,
            },
            direction(&["decrypt", "encrypt"]),
        ],
    },
};

/// Signature: verifies on receive, signs on send.
pub const SIGNATURE_UNIT: &Declaration = &Declaration {
    name: SIGNATURE,
    capability: "prepare",
    stages: EVERY_STAGE,
    settings: &Settings {
        technology: SIGNATURE,
        settings: &[
            FORMAT,
            ENGINE,
            Setting {
                name: "key",
                kind: Kind::Address,
                presence: Presence::Optional,
                meaning: "This end's private key, which signs on send.",
                applies: Applies::Both,
            },
            Setting {
                name: "signers",
                kind: Kind::Text,
                presence: Presence::Optional,
                meaning: "The far ends' public keys or certificates, files separated by commas, which verify on receive.",
                applies: Applies::Both,
            },
            direction(&["verify", "sign"]),
        ],
    },
};

/// Every standard prepare unit's declaration, in the order a person reads
/// them.
pub const DECLARATIONS: [&Declaration; 6] = [
    COMPRESSION_UNIT,
    CHARACTER_ENCODING_UNIT,
    TRANSFER_ENCODING_UNIT,
    TEXT_NORMALIZATION_UNIT,
    ENCRYPTION_UNIT,
    SIGNATURE_UNIT,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declaration_is_sound_and_a_prepare_unit() {
        for declaration in DECLARATIONS {
            assert!(
                declaration.problems().is_empty(),
                "{:?}",
                declaration.problems()
            );
            assert_eq!(declaration.capability, "prepare");
        }
    }
}
