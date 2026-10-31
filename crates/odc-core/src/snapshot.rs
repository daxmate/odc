//! Streaming codec for the commit index snapshot.
//!
//! A snapshot is a canonical CBOR array of `[word, digest]` pairs stored
//! zstd-compressed in `commit_index`. The writer and reader here encode and
//! decode one pair at a time so a million-entry index never has to exist as a
//! single in-memory buffer.

use crate::cbor;
use crate::error::{Error, Result};
use std::io::{BufReader, Read, Write};

/// Writes a snapshot one `(word, digest)` pair at a time.
pub struct SnapshotWriter<W: Write> {
    inner: W,
    raw_size: u64,
    written: u64,
}

impl<W: Write> SnapshotWriter<W> {
    /// Start a snapshot holding exactly `count` pairs.
    ///
    /// `count` must match the number of [`SnapshotWriter::push`] calls; the
    /// header is written up front so the result is byte-identical to
    /// `cbor::encode(&Value::Array(..))` of the same pairs.
    pub fn new(inner: W, count: u64) -> Result<Self> {
        let mut writer = Self {
            inner,
            raw_size: 0,
            written: 0,
        };
        let mut header = Vec::with_capacity(9);
        cbor::write_array_header(count, &mut header);
        writer.write_raw(&header)?;
        Ok(writer)
    }

    /// Append one pair.
    pub fn push(&mut self, word: &str, digest: &[u8; 32]) -> Result<()> {
        let mut buffer = Vec::with_capacity(word.len() + 40);
        cbor::write_array_header(2, &mut buffer);
        cbor::write_text(word, &mut buffer);
        cbor::write_bytes(digest, &mut buffer);
        self.write_raw(&buffer)?;
        self.written += 1;
        Ok(())
    }

    /// Uncompressed CBOR bytes written so far.
    #[must_use]
    pub fn raw_size(&self) -> u64 {
        self.raw_size
    }

    /// Pairs written so far.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.written
    }

    /// True when no pair has been written.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.written == 0
    }

    /// Flush and hand the underlying writer back, with the raw byte count.
    pub fn finish(mut self) -> Result<(W, u64)> {
        self.inner.flush()?;
        Ok((self.inner, self.raw_size))
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.write_all(bytes)?;
        self.raw_size += bytes.len() as u64;
        Ok(())
    }
}

/// Reads a snapshot one `(word, digest)` pair at a time.
pub struct SnapshotReader<R: Read> {
    inner: BufReader<R>,
    remaining: u64,
    bytes: u64,
}

impl<R: Read> SnapshotReader<R> {
    /// Read the array header and prepare to decode `remaining` pairs.
    pub fn new(inner: R) -> Result<Self> {
        let mut inner = BufReader::with_capacity(64 * 1024, inner);
        let (major, count) = read_head(&mut inner)?;
        if major != 4 {
            return Err(Error::Cbor("snapshot is not a CBOR array"));
        }
        Ok(Self {
            inner,
            remaining: count,
            bytes: head_len(count),
        })
    }

    /// Decode the next pair, or `None` once the declared count is exhausted.
    pub fn next_pair(&mut self) -> Result<Option<(String, [u8; 32])>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        let (major, length) = read_head(&mut self.inner)?;
        if major != 4 || length != 2 {
            return Err(Error::Cbor("snapshot item is not a [word, digest] pair"));
        }
        self.bytes += head_len(length);
        let (major, length) = read_head(&mut self.inner)?;
        if major != 3 {
            return Err(Error::Cbor("snapshot word is not text"));
        }
        self.bytes += head_len(length);
        let length =
            usize::try_from(length).map_err(|_| Error::Cbor("snapshot word is too long"))?;
        let mut word = vec![0u8; length];
        self.inner.read_exact(&mut word)?;
        self.bytes += length as u64;
        let word =
            String::from_utf8(word).map_err(|_| Error::Cbor("snapshot word is not UTF-8"))?;
        let (major, length) = read_head(&mut self.inner)?;
        if major != 2 || length != 32 {
            return Err(Error::Cbor("snapshot digest is not a 32-byte string"));
        }
        self.bytes += head_len(length) + length;
        let mut digest = [0u8; 32];
        self.inner.read_exact(&mut digest)?;
        Ok(Some((word, digest)))
    }

    /// Pairs still to be decoded.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Uncompressed CBOR bytes consumed so far.
    ///
    /// Compare with the stored `raw_size` to detect truncated or padded
    /// commit indexes without decompressing the blob again.
    #[must_use]
    pub fn bytes_read(&self) -> u64 {
        self.bytes
    }
}

/// Read one CBOR head: major type plus argument.
fn read_head<R: Read>(reader: &mut R) -> Result<(u8, u64)> {
    let mut first = [0u8; 1];
    reader.read_exact(&mut first)?;
    let major = first[0] >> 5;
    let argument = match first[0] & 0x1f {
        small @ 0..=23 => u64::from(small),
        24 => {
            let mut bytes = [0u8; 1];
            reader.read_exact(&mut bytes)?;
            u64::from(bytes[0])
        }
        25 => {
            let mut bytes = [0u8; 2];
            reader.read_exact(&mut bytes)?;
            u64::from(u16::from_be_bytes(bytes))
        }
        26 => {
            let mut bytes = [0u8; 4];
            reader.read_exact(&mut bytes)?;
            u64::from(u32::from_be_bytes(bytes))
        }
        27 => {
            let mut bytes = [0u8; 8];
            reader.read_exact(&mut bytes)?;
            u64::from_be_bytes(bytes)
        }
        31 => return Err(Error::Cbor("indefinite length items are not allowed")),
        _ => return Err(Error::Cbor("reserved additional information")),
    };
    Ok((major, argument))
}

/// Encoded width of a CBOR head carrying `argument`.
///
/// Mirrors the minimal-length rule in `cbor::push_head`, so byte accounting
/// stays exact without re-encoding the value.
fn head_len(argument: u64) -> u64 {
    match argument {
        0..=23 => 1,
        24..=0xff => 2,
        0x100..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}
