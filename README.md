# xmip-core-prepare

Preparation: the standard prepare units, each working on a Stream's bytes
and never on a Message (`doc/architecture/runtime-model.md` section 8) —
before a Message exists on receive, after serialization on send. A
transformation is not a prepare: turning one Message's content into a new
Message's is `xmip-core-transform`'s.

A unit is one function for both ways, opened for the stage its step stands
at — a Receive Preparation, a Process Preparation or a Send Preparation —
and handed that stage (the owner, 2026-10-22: *It is just an enum away what
it does in what situation*). On receive it undoes what the far end did, on
send it does it; at process, or wherever a step says so, its `direction`
decides (`Direction`). Each is declared once beside its code as an
`xmip-core-node` `unit::Unit` — name, capability, stages, settings in
xmip-core's one shape — and opened through that declaration, so the
standard module's manifest (`xmip-core-unit`) is written from it.

| Unit | Does | Settings |
| --- | --- | --- |
| `Compression` | decompresses on receive, compresses on send | `format` gzip, zlib, deflate, zstd or zip; `level` fastest, default, best; `entry`, a zip entry's name; `direction` |
| `CharacterEncoding` | reads the far end's encoding into UTF-8 on receive, writes it on send | `encoding`, any WHATWG label or `utf-16le`, `utf-16be`, `utf-32le`, `utf-32be`; `direction` |
| `TransferEncoding` | decodes Base 64 on receive, encodes it on send | `format` base64 or base64url; `line_length`; `direction` |
| `TextNormalization` | keeps, removes or adds the byte order mark, rewrites line endings | `byte_order_mark`; `line_ending` |
| `Encryption` | decrypts on receive, encrypts on send | `format` openpgp, cms, jose; `engine`; `key`; `recipients`; `direction` |
| `Signature` | verifies on receive, signs on send | `format`; `engine`; `key`; `signers`; `direction` |

**Every unit streams.** A unit reads its input a piece at a time and writes
its output as it goes, so none holds the whole Stream: `tests/bounded.rs`
carries 64 MiB through each both ways, the two ends joined by a pipe, with
what the process holds counted (`xcore::held`) and under 16 MiB, and a
kilobyte through each within the millisecond rule.

**Strict.** Character Encoding refuses bytes that are not the named
encoding and a character it cannot write, never replacing one (ADR-0038,
amendment 2026-09-26); Transfer Encoding refuses a character outside its
alphabet and anything after the padding; Compression refuses what is not
compressed as named and a zip entry that does not match its check.

**Its dependencies, each chosen.** gzip, zlib and raw deflate are
`flate2`'s over its pure-Rust backend; zstd is `ruzstd`'s, Rust both ways,
where the `zstd` crate binds the C library; a zip entry is read and written
here (`zip.rs`) over `flate2`, one entry being one Stream; the WHATWG
encodings are `encoding_rs`'s; Base 64 and the Unicode forms are
`xmip-core-library-codec`'s. None reaches a network (ADR-0045, amendment
2026-10-22).

**Engines.** Encryption and Signature work through an engine of the format
a step names, chosen by its `engine`: a technology of this capability
behind `protection::ProtectionEngine`, mounted beside `.src` (ADR-0049) and
handed in by the module that links it. The first is `rpgp`, for OpenPGP.
CMS and JOSE have no engine yet, and a step naming one is refused when it
is opened. Keys are files the step names; nothing is looked up over a
network — no key server, no online revocation check.

`doc/architecture/runtime-model.md` section 8 governs Preparation; ADR-0066,
amendments 2026-10-22, the units; `architecture.toml` carries the maturity.
