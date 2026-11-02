//! The module's timing program (`doc/governance/rust-style.md` section
//! 5c): the prepare units held to the millisecond rule (`CONTRIBUTING.md`:
//! over a millisecond beyond load is a defect). A plain `cargo test` leaves
//! it out; the landing runs it after every other test of the module,
//! alone, one test at a time (`cargo test --test measured --all-features
//! -- --test-threads=1`), so what is measured is Xmip's work and not its
//! neighbors':
//!
//! - every standard unit carries a small Stream both ways well within a
//!   millisecond, over what `tests/bounded.rs` carries (`units.rs`,
//!   shared).

mod units;

use std::time::{Duration, Instant};

use units::{CARRIED, Made, Sink, given};
use xmip_core_prepare::{Stage, open};

#[test]
fn every_unit_carries_a_small_stream_at_once() {
    for (unit, settings) in CARRIED {
        let settings: Vec<(&str, &str)> = settings.to_vec();
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
