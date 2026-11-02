//! Every standard prepare unit carries a Stream far larger than the pieces
//! it holds in bounded memory. How fast a small one is carried is the
//! module's timing program's (`tests/measured`).
//!
//! One test in a program of its own: the allocator's count is the whole
//! process's, so nothing runs beside it.

#[path = "measured/units.rs"]
mod units;

use std::io;
use std::thread;
use std::time::Instant;

use units::{CARRIED, Made, Sink, given};
use xmip_core_prepare::{Stage, open};

#[global_allocator]
static HELD: xcore::held::Counted = xcore::held::Counted;

/// The Stream carried: 64 MiB of text in CR LF lines.
const LENGTH: u64 = 64 * 1024 * 1024;

/// What a unit may hold however long the Stream: a few pieces, and the
/// window a compressor keeps.
const BOUND: usize = 16 * 1024 * 1024;

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

#[test]
fn every_unit_carries_a_large_stream_in_bounded_memory() {
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
    }
}
