//! How much decompressing may make of one Stream: an output ceiling and a
//! ratio, both a step's settings, so a compression bomb — a kilobyte that
//! inflates to gigabytes — is refused while it is still being read.
//!
//! The ceiling is the most bytes one Stream may come to; the ratio, the most
//! bytes each compressed byte may make, is held once the first mebibyte has
//! come out, so a small Stream that compresses well is never refused for it.
//! Their defaults (1 GiB and 200 to 1) are defaults only: a step sets its
//! own where its partner's Streams are larger or compress better. Both are
//! refused through `codec::ceiling::within`, the estate's one sentence for a
//! size over what carries it, below the network so a unit working offline
//! reaches none.
//!
//! Every inflated byte is counted — a gzip member's, a zip entry's whether
//! it is the one chosen or one passed over, zlib's and raw deflate's in
//! `crate::deflate::inflate`, zstd's through [`Compressed`] and [`Bounded`].

use std::cell::Cell;
use std::io::{self, Read, Write};

use codec::ceiling;

use crate::Values;

/// The bytes out before the ratio is held.
const RATIO_FLOOR: u64 = 1 << 20;

/// A step's limit on decompressing, and what one Stream has read and made
/// against it so far: made for each Stream, since a step is shared between
/// threads and its counts are not.
pub(crate) struct Tally {
    ceiling: u64,
    ratio: u64,
    read: Cell<u64>,
    made: Cell<u64>,
}

/// A step's limit: its `ceiling` and its `ratio`, as its settings say.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Limit {
    ceiling: u64,
    ratio: u64,
}

impl Limit {
    pub(crate) fn of(values: &Values) -> Self {
        let setting = |name| u64::try_from(values.integer(name)).unwrap_or(1).max(1);
        Self::new(setting("ceiling"), setting("ratio"))
    }

    pub(crate) const fn new(ceiling: u64, ratio: u64) -> Self {
        Self { ceiling, ratio }
    }
}

impl Tally {
    /// Nothing read or made yet, against `limit`: one Stream's.
    pub(crate) const fn new(limit: Limit) -> Self {
        Self {
            ceiling: limit.ceiling,
            ratio: limit.ratio,
            read: Cell::new(0),
            made: Cell::new(0),
        }
    }

    /// `read` more compressed bytes made `made` more.
    ///
    /// # Errors
    /// The Stream has come to more than the ceiling, or past the first
    /// mebibyte to more than the ratio allows of what it read.
    pub(crate) fn add(&self, read: usize, made: usize) -> io::Result<()> {
        let read = self.read.get().saturating_add(read as u64);
        let made = self.made.get().saturating_add(made as u64);
        self.read.set(read);
        self.made.set(made);
        let size = usize::try_from(made).unwrap_or(usize::MAX);
        let bytes = |limit: u64| usize::try_from(limit).unwrap_or(usize::MAX);
        ceiling::within(size, bytes(self.ceiling), "bytes this step decompresses")
            .and_then(|()| {
                if made <= RATIO_FLOOR {
                    return Ok(());
                }
                let allowed = read.saturating_mul(self.ratio);
                let carrier = format!("{read} compressed bytes make at {} to 1", self.ratio);
                ceiling::within(size, bytes(allowed), &carrier)
            })
            .map_err(|over| io::Error::new(io::ErrorKind::InvalidData, over.message))
    }
}

/// The compressed bytes read, counted against a [`Tally`].
pub(crate) struct Compressed<'a> {
    pub(crate) inner: &'a mut dyn Read,
    pub(crate) tally: &'a Tally,
}

impl Read for Compressed<'_> {
    fn read(&mut self, into: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(into)?;
        self.tally.add(read, 0)?;
        Ok(read)
    }
}

/// The bytes decompressing makes, counted against a [`Tally`] before they
/// are written.
pub(crate) struct Bounded<'a> {
    pub(crate) inner: &'a mut dyn Write,
    pub(crate) tally: &'a Tally,
}

impl Write for Bounded<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.tally.add(0, bytes.len())?;
        self.inner.write_all(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ceiling_holds_always_and_the_ratio_past_the_floor() {
        let limit = Limit {
            ceiling: 4 << 20,
            ratio: 10,
        };
        let tally = Tally::new(limit);
        tally
            .add(10, 1 << 20)
            .expect("under the floor, whatever the ratio");
        let over = tally.add(0, 1).expect_err("past the floor at 100 000 to 1");
        assert!(
            over.to_string()
                .contains("10 compressed bytes make at 10 to 1")
        );
        let tally = Tally::new(limit);
        tally.add(1 << 20, 4 << 20).expect("at the ceiling");
        let over = tally.add(1 << 20, 1).expect_err("one byte over");
        assert_eq!(
            over.to_string(),
            "4194305 bytes is over the 4194304 bytes this step decompresses"
        );
    }
}
