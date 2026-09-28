//! A byte-stream view of a `blk` device: `embedded_io::{Read, Write, Seek}`, bounced through one
//! sector-sized buffer per transfer. `hadris_io`'s own `Read`/`Write`/`Seek` (and so
//! `hadris_fat::FatVolume::open`) are implemented for anything implementing `embedded_io`'s,
//! *without* the `hadris-fat`/`hadris-io` `std` feature enabled (see `hadris_io::sync_api`'s
//! blanket impls) -- which is how this crate is configured (`Cargo.toml`), so `BlkIo` needs no
//! `hadris_io`-specific code at all.
//!
//! Every transfer is capped to what fits in the sector `sector::plan` says it starts in -- never
//! more than one sector per `read`/`write` call. `embedded_io::Read::read_exact`/
//! `Write::write_all` (which `hadris-fat` uses for anything longer) already loop calling `read`/
//! `write` until done, so nothing here needs its own multi-sector loop.

use super::sector::{self, Transfer};
use crate::drivers::virtio::blk::Blk;

/// What can go wrong talking to the block device through this byte-stream view.
#[derive(Debug)]
pub enum Error {
    /// The underlying `virtio-blk` transfer failed.
    Device(virtio_drivers::Error),
    /// A seek landed at a negative position, or a write was attempted at or past the device's own
    /// (fixed, unextendable) capacity.
    OutOfRange,
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Device(e) => write!(f, "block device error: {e:?}"),
            Error::OutOfRange => write!(f, "position out of range"),
        }
    }
}

impl core::error::Error for Error {}

impl embedded_io::Error for Error {
    fn kind(&self) -> embedded_io::ErrorKind {
        match self {
            Error::Device(_) => embedded_io::ErrorKind::Other,
            Error::OutOfRange => embedded_io::ErrorKind::InvalidInput,
        }
    }
}

/// A `blk` device, read/written a byte range at a time instead of a sector at a time.
pub struct BlkIo {
    dev: &'static mut Blk,
    position: u64,
}

impl BlkIo {
    pub fn new(dev: &'static mut Blk) -> Self {
        Self { dev, position: 0 }
    }

    /// Reads sector `sector` whole, into a fresh sector-sized buffer.
    fn read_sector(&mut self, sector: usize) -> Result<[u8; sector::SECTOR_SIZE], Error> {
        let mut block = [0u8; sector::SECTOR_SIZE];
        self.dev
            .read_blocks(sector, &mut block)
            .map_err(Error::Device)?;
        Ok(block)
    }
}

impl embedded_io::ErrorType for BlkIo {
    type Error = Error;
}

impl embedded_io::Read for BlkIo {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        let remaining = self.dev.capacity_bytes().saturating_sub(self.position);
        if remaining == 0 {
            return Ok(0); // EOF
        }
        let want = remaining.min(buf.len() as u64) as usize;
        let Transfer {
            sector,
            offset,
            len,
        } = sector::plan(self.position, want);
        let block = self.read_sector(sector)?;
        buf[..len].copy_from_slice(&block[offset..offset + len]);
        self.position += len as u64;
        Ok(len)
    }
}

impl embedded_io::Write for BlkIo {
    fn write(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.position >= self.dev.capacity_bytes() {
            return Err(Error::OutOfRange);
        }
        let remaining = self.dev.capacity_bytes() - self.position;
        let want = remaining.min(buf.len() as u64) as usize;
        let Transfer {
            sector,
            offset,
            len,
        } = sector::plan(self.position, want);
        // Read-modify-write: preserve whatever this sector already holds outside this transfer's
        // own range, rather than assuming (and zeroing) the rest of a partially-written sector.
        let mut block = self.read_sector(sector)?;
        block[offset..offset + len].copy_from_slice(&buf[..len]);
        self.dev
            .write_blocks(sector, &block)
            .map_err(Error::Device)?;
        self.position += len as u64;
        Ok(len)
    }

    fn flush(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl embedded_io::Seek for BlkIo {
    fn seek(&mut self, pos: embedded_io::SeekFrom) -> Result<u64, Error> {
        let new_position = match pos {
            embedded_io::SeekFrom::Start(n) => Some(n),
            embedded_io::SeekFrom::End(n) => self.dev.capacity_bytes().checked_add_signed(n),
            embedded_io::SeekFrom::Current(n) => self.position.checked_add_signed(n),
        };
        self.position = new_position.ok_or(Error::OutOfRange)?;
        Ok(self.position)
    }
}
