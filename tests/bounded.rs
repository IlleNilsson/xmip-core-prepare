//! Every standard prepare unit carries a Stream far larger than the pieces
//! it holds in bounded memory, and a small one well within the millisecond
//! rule (CONTRIBUTING.md: over a millisecond beyond load is a defect).
//!
//! One test in a program of its own: the allocator's count is the whole
//! process's, so nothing runs beside it.

use std::io::{self, Read, Write};
use std::thread;
use std::time::{Duration, Instant};

use xcore::settings::Given;
use xmip_core_prepare::{Stage, open};

#[global_allocator]
static HELD: xcore::held::Counted = xcore::held::Counted;

/// The Stream carried: 64 MiB of text in CR LF lines.
const LENGTH: u64 = 64 * 1024 * 1024;

/// What a unit may hold however long the Stream: a few pieces, and the
/// window a compressor keeps.
const BOUND: usize = 16 * 1024 * 1024;

/// Text made as it is read, `length` bytes: lines of letters, each ended by
/// CR LF.
struct Made {
    at: u64,
    length: u64,
}

impl Read for Made {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let left = usize::try_from(self.length - self.at).unwrap_or(usize::MAX);
        let taken = out.len().min(left);
        for byte in &mut out[..taken] {
            *byte = match self.at % 64 {
                62 => b'\r',
                63 => b'\n',
                at => b'a' + u8::try_from(at % 26).unwrap_or_default(),
            };
            self.at += 1;
        }
        Ok(taken)
    }
}

/// What was written, counted and summed, never kept.
#[derive(Default)]
struct Sink {
    length: u64,
    sum: u64,
}

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.length += bytes.len() as u64;
        for &byte in bytes {
            self.sum = self.sum.wrapping_mul(31).wrapping_add(u64::from(byte));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn given(settings: &[(&str, &str)]) -> Vec<(String, Given)> {
    settings
        .iter()
        .map(|(name, value)| {
            let value = value
                .parse()
                .map_or_else(|_| Given::Text((*value).to_string()), Given::Integer);
            ((*name).to_string(), value)
        })
        .collect()
}

/// `length` bytes taken out by `unit` at send and back in at receive, the
/// two ends joined by a pipe so neither end's output is ever whole.
fn both_ways(unit: &str, settings: &[(&str, &str)], length: u64) -> Sink {
    let outward = open(unit, Stage::Send, &given(settings)).expect("opens at send");
    let inward = open(unit, Stage::Receive, &given(settings)).expect("opens at receive");
    let (mut reading, mut writing) = io::pipe().expect("a pipe");
    let sending = thread::spawn(move || {
        outward
            .prepare(&mut Made { at: 0, length }, &mut writing)
            .expect("prepared at send");
    });
    let mut sink = Sink::default();
    inward
        .prepare(&mut reading, &mut sink)
        .expect("prepared at receive");
    sending.join().expect("the send end");
    sink
}

/// The units and settings carried, each both ways.
const CARRIED: &[(&str, &[(&str, &str)])] = &[
    ("Compression", &[("format", "gzip")]),
    ("Compression", &[("format", "zlib")]),
    ("Compression", &[("format", "deflate")]),
    ("Compression", &[("format", "zstd")]),
    ("Compression", &[("format", "zip")]),
    ("CharacterEncoding", &[("encoding", "utf-16le")]),
    ("CharacterEncoding", &[("encoding", "windows-1252")]),
    ("TransferEncoding", &[("line_length", "76")]),
    ("TextNormalization", &[("line_ending", "lf")]),
];

#[test]
fn every_unit_carries_a_large_stream_in_bounded_memory_and_a_small_one_at_once() {
    let mut expected = Sink::default();
    io::copy(
        &mut Made {
            at: 0,
            length: LENGTH,
        },
        &mut expected,
    )
    .expect("made");
    for (unit, settings) in CARRIED {
        let settings: Vec<(&str, &str)> = settings.to_vec();
        let started = Instant::now();
        let (held, sink) = xcore::held::above(|| both_ways(unit, &settings, LENGTH));
        println!(
            "{unit} {settings:?}: 64 MiB both ways in {:?}, {} KiB held at most",
            started.elapsed(),
            held / 1024
        );
        assert!(held < BOUND, "{unit} {settings:?} held {held} bytes");
        if *unit != "TextNormalization" {
            assert_eq!(
                (sink.length, sink.sum),
                (expected.length, expected.sum),
                "{unit}"
            );
        }
        // The small Stream measures the units' own work, as a Message pays
        // it: each step opened once, as a node opens it, and no thread or
        // pipe of the harness's in the figure.
        let outward = open(unit, Stage::Send, &given(&settings)).expect("opens at send");
        let inward = open(unit, Stage::Receive, &given(&settings)).expect("opens at receive");
        let mut prepared = Vec::new();
        let rounds: Vec<Duration> = (0..50)
            .map(|_| {
                prepared.clear();
                let started = Instant::now();
                outward
                    .prepare(
                        &mut Made {
                            at: 0,
                            length: 1024,
                        },
                        &mut prepared,
                    )
                    .expect("prepared at send");
                inward
                    .prepare(&mut &prepared[..], &mut Sink::default())
                    .expect("prepared at receive");
                started.elapsed()
            })
            .collect();
        let spread = xcore::latency::spread(&format!("{unit} {settings:?}, 1 KiB"), rounds);
        assert!(
            spread.fastest_of_five < Duration::from_millis(1),
            "{unit} {settings:?}: {spread:?}"
        );
    }
}
