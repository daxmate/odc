//! Streaming codec for commit index entries.
//!
//! An entry is a CBOR array stored zstd-compressed in `commit_index`:
//!
//! ```text
//! ["odc-idx", version, kind, depth, entry_count, body]
//! ```
//!
//! The leading text string cannot be confused with a pair array, which is how
//! entries written before this frame are recognised. `version` is `1` for
//! entries written by this module;
//! * `kind` is `0` for a full snapshot and `1` for a delta;
//! * `depth` counts consecutive deltas since the nearest full entry (0 for a
//!   full entry);
//! * `entry_count` is the number of pairs the entry rebuilds;
//! * `body` is the array of `[word, digest]` pairs for a full entry, or
//!   the array of [`Op`]s for a delta.
//!
//! Entries written before this frame (a bare CBOR array of pairs) are still
//! accepted and read as full snapshots, so old repositories open without a
//! migration.
//!
//! The writer and reader encode and decode one item at a time so a
//! million-entry index never has to exist as a single in-memory buffer. A delta
//! is replayed over its parent by [`DeltaSource`], which merges two sorted
//! streams and therefore holds one pair from each side, not the whole index.

use crate::cbor;
use crate::error::{Error, Result};
use std::cmp::Ordering;
use std::io::{BufReader, Read, Write};

/// Format version written in the frame of every new entry.
pub const ENTRY_VERSION: u64 = 1;

/// First element of a framed entry. Entries written before framing start with
/// their first `[word, digest]` pair and are read as full snapshots.
const MAGIC: &str = "odc-idx";

/// Number of consecutive deltas after which a full snapshot is written.
///
/// A commit stores the difference against its parent, so rebuilding an index
/// walks back to the nearest full entry. Anchoring every 32 commits bounds that
/// walk to 31 delta decodes. The cost is one full snapshot per 32 commits: on a
/// history with a 1.5 MiB full snapshot that is ~47 KiB per commit on average,
/// against 1.5 MiB per commit when every entry is full.
pub const ANCHOR_INTERVAL: u32 = 32;

/// Hard bound on how many deltas a read will chain before giving up.
///
/// A repository written by this implementation never exceeds
/// [`ANCHOR_INTERVAL`, but the limit also protects reads against a
/// hand-edited or corrupted chain that would otherwise recurse forever.
pub const MAX_CHAIN_DEPTH: u32 = ANCHOR_INTERVAL + 8;

/// Shape of a commit index entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// Complete list of pairs; a read starts here.
    Full,
    /// Difference against the parent commit's index.
    Delta,
}

/// Header shared by every entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryHeader {
    /// Whether the body is a full snapshot or a delta.
    pub kind: EntryKind,
    /// Consecutive deltas between this entry and the nearest full entry.
    pub depth: u32,
    /// Number of pairs the entry rebuilds (also the full body length).
    pub entry_count: u64,
    /// Number of body items: pairs for a full entry, operations for a delta.
    pub body_count: u64,
}

/// One difference against the parent index.
///
/// Operations are stored in strictly ascending `word` order so a reader
/// can replay them against the parent stream without buffering either side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Word absent from the parent index.
    Add {
        /// Normalised key.
        word: String,
        /// Full 32-byte payload digest.
        digest: [u8; 32],
    },
    /// Word present in both indexes with a different digest.
    Change {
        /// Normalised key.
        word: String,
        /// Full 32-byte payload digest.
        digest: [u8; 32],
    },
    /// Word present in the parent index and absent from this one.
    Remove {
        /// Normalised key.
        word: String,
    },
}

impl Op {
    /// Normalised key the operation applies to.
    #[must_use]
    pub fn word(&self) -> &str {
        match self {
            Self::Add { word, .. } | Self::Change { word, .. } | Self::Remove { word } => word,
        }
    }

    fn code(&self) -> u64 {
        match self {
            Self::Add { .. } => 0,
            Self::Change { .. } => 1,
            Self::Remove { .. } => 2,
        }
    }
}

/// One decoded commit index, in ascending word order.
pub trait PairSource {
    /// Next pair, or `None` once the index is exhausted.
    fn next_pair(&mut self) -> Result<Option<(String, [u8; 32])>>;

    /// Uncompressed CBOR bytes consumed so far, including the parent chain.
    fn bytes_read(&self) -> u64;
}

/// Writes a commit index entry one item at a time.
pub struct SnapshotWriter<W: Write> {
    inner: W,
    raw_size: u64,
    remaining: u64,
    kind: EntryKind,
}

impl<W: Write> SnapshotWriter<W> {
    /// Start a full entry holding exactly `entry_count` pairs.
    pub fn full(inner: W, entry_count: u64) -> Result<Self> {
        Self::framed(inner, EntryKind::Full, 0, entry_count, entry_count)
    }

    /// Start a delta holding exactly `op_count` operations that rebuilds
    /// `entry_count` pairs from a parent `depth - 1` levels away.
    pub fn delta(inner: W, depth: u32, entry_count: u64, op_count: u64) -> Result<Self> {
        Self::framed(inner, EntryKind::Delta, depth, entry_count, op_count)
    }

    fn framed(
        inner: W,
        kind: EntryKind,
        depth: u32,
        entry_count: u64,
        body_count: u64,
    ) -> Result<Self> {
        let mut writer = Self {
            inner,
            raw_size: 0,
            remaining: body_count,
            kind,
        };
        let mut header = Vec::with_capacity(24);
        cbor::write_array_header(6, &mut header);
        cbor::write_text(MAGIC, &mut header);
        cbor::write_uint(ENTRY_VERSION, &mut header);
        cbor::write_uint(
            match kind {
                EntryKind::Full => 0,
                EntryKind::Delta => 1,
            },
            &mut header,
        );
        cbor::write_uint(u64::from(depth), &mut header);
        cbor::write_uint(entry_count, &mut header);
        cbor::write_array_header(body_count, &mut header);
        writer.write_raw(&header)?;
        Ok(writer)
    }

    /// Append one `(word, digest)` pair to a full entry.
    pub fn push(&mut self, word: &str, digest: &[u8; 32]) -> Result<()> {
        if self.kind != EntryKind::Full {
            return Err(Error::Inconsistent(
                "snapshot writer: pair pushed to a delta entry".to_string(),
            ));
        }
        if self.remaining == 0 {
            return Err(Error::Inconsistent(
                "snapshot writer: more pairs than declared".to_string(),
            ));
        }
        let mut buffer = Vec::with_capacity(word.len() + 40);
        cbor::write_array_header(2, &mut buffer);
        cbor::write_text(word, &mut buffer);
        cbor::write_bytes(digest, &mut buffer);
        self.write_raw(&buffer)?;
        self.remaining -= 1;
        Ok(())
    }

    /// Append one operation to a delta entry.
    pub fn push_op(&mut self, op: &Op) -> Result<()> {
        if self.kind != EntryKind::Delta {
            return Err(Error::Inconsistent(
                "snapshot writer: operation pushed to a full entry".to_string(),
            ));
        }
        if self.remaining == 0 {
            return Err(Error::Inconsistent(
                "snapshot writer: more operations than declared".to_string(),
            ));
        }
        let mut buffer = Vec::with_capacity(op.word().len() + 40);
        let width = match op {
            Op::Add { .. } | Op::Change { .. } => 3,
            Op::Remove { .. } => 2,
        };
        cbor::write_array_header(width, &mut buffer);
        cbor::write_text(op.word(), &mut buffer);
        cbor::write_uint(op.code(), &mut buffer);
        if let Op::Add { digest, .. } | Op::Change { digest, .. } = op {
            cbor::write_bytes(digest, &mut buffer);
        }
        self.write_raw(&buffer)?;
        self.remaining -= 1;
        Ok(())
    }

    /// Flush the entry and return the inner writer with its raw byte count.
    pub fn finish(mut self) -> Result<(W, u64)> {
        if self.remaining != 0 {
            return Err(Error::Inconsistent(
                "snapshot writer: fewer items than declared".to_string(),
            ));
        }
        self.inner.flush()?;
        Ok((self.inner, self.raw_size))
    }

    fn write_raw(&mut self, bytes: &[u8]) -> Result<()> {
        self.inner.write_all(bytes)?;
        self.raw_size += bytes.len() as u64;
        Ok(())
    }
}

/// Reads one commit index entry.
pub struct SnapshotReader<R: Read> {
    inner: BufReader<R>,
    header: EntryHeader,
    remaining: u64,
    bytes: u64,
    last_word: Option<String>,
    /// Head already read while recognising the frame (legacy six-pair case).
    pending: Option<(u8, u64)>,
}

impl<R: Read> SnapshotReader<R> {
    /// Read the entry frame and prepare to decode its body.
    ///
    /// Bare arrays written before the version frame are decoded as full
    /// entries, which keeps every existing repository readable.
    pub fn new(inner: R) -> Result<Self> {
        let mut reader = Self {
            inner: BufReader::with_capacity(64 * 1024, inner),
            header: EntryHeader {
                kind: EntryKind::Full,
                depth: 0,
                entry_count: 0,
                body_count: 0,
            },
            remaining: 0,
            bytes: 0,
            last_word: None,
            pending: None,
        };
        let header = reader.read_header()?;
        reader.remaining = header.body_count;
        reader.header = header;
        Ok(reader)
    }

    /// Frame of this entry.
    #[must_use]
    pub fn header(&self) -> EntryHeader {
        self.header
    }

    fn read_header(&mut self) -> Result<EntryHeader> {
        let (major, count) = self.read_head()?;
        if major != 4 {
            return Err(Error::Cbor("snapshot entry is not a CBOR array"));
        }
        if count != 6 {
            // Legacy entry: a bare array of `[word, digest]` pairs.
            return Ok(EntryHeader {
                kind: EntryKind::Full,
                depth: 0,
                entry_count: count,
                body_count: count,
            });
        }
        let (second_major, second_argument) = self.read_head()?;
        if second_major == 4 && second_argument == 2 {
            // A legacy snapshot that happens to hold six pairs: the head just
            // read belongs to the first pair.
            self.pending = Some((second_major, second_argument));
            return Ok(EntryHeader {
                kind: EntryKind::Full,
                depth: 0,
                entry_count: count,
                body_count: count,
            });
        }
        if second_major != 3 {
            return Err(Error::Cbor("snapshot entry has no recognised version"));
        }
        let magic = self.read_text_body(second_argument)?;
        if magic != MAGIC {
            return Err(Error::Cbor("snapshot entry has no recognised version"));
        }
        let version = self.read_uint()?;
        if version != ENTRY_VERSION {
            return Err(Error::Cbor("snapshot entry version is unsupported"));
        }
        let kind = match self.read_uint()? {
            0 => EntryKind::Full,
            1 => EntryKind::Delta,
            _ => return Err(Error::Cbor("snapshot entry kind is unknown")),
        };
        let depth = u32::try_from(self.read_uint()?)
            .map_err(|_| Error::Cbor("snapshot depth is too large"))?;
        if depth > MAX_CHAIN_DEPTH {
            return Err(Error::Inconsistent(format!(
                "commit index chain is deeper than {MAX_CHAIN_DEPTH} deltas"
            )));
        }
        let entry_count = self.read_uint()?;
        let body = self.read_head()?;
        if body.0 != 4 {
            return Err(Error::Cbor("snapshot entry body is not an array"));
        }
        if kind == EntryKind::Full && body.1 != entry_count {
            return Err(Error::Cbor("snapshot body does not match entry count"));
        }
        Ok(EntryHeader {
            kind,
            depth,
            entry_count,
            body_count: body.1,
        })
    }

    /// Decode the next pair of a full entry.
    pub fn next_pair(&mut self) -> Result<Option<(String, [u8; 32])>> {
        if self.header.kind != EntryKind::Full {
            return Err(Error::Cbor("snapshot entry is a delta, not a pair list"));
        }
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        let (major, length) = self.read_head()?;
        if major != 4 || length != 2 {
            return Err(Error::Cbor("snapshot item is not a [word, digest] pair"));
        }
        let word = self.read_text()?;
        let digest = self.read_digest()?;
        Ok(Some((word, digest)))
    }

    /// Decode the next operation of a delta entry.
    pub fn next_op(&mut self) -> Result<Option<Op>> {
        if self.header.kind != EntryKind::Delta {
            return Err(Error::Cbor("snapshot entry is full, not a delta"));
        }
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        let (major, width) = self.read_head()?;
        if major != 4 || !(2..=3).contains(&width) {
            return Err(Error::Cbor("snapshot operation is not an array"));
        }
        let word = self.read_text()?;
        if let Some(last) = &self.last_word {
            if word.as_str() <= last.as_str() {
                return Err(Error::Inconsistent(format!(
                    "commit index delta is not strictly sorted at {word}"
                )));
            }
        }
        self.last_word = Some(word.clone());
        let code = self.read_uint()?;
        let op = match (code, width) {
            (0, 3) => Op::Add {
                digest: self.read_digest()?,
                word,
            },
            (1, 3) => Op::Change {
                digest: self.read_digest()?,
                word,
            },
            (2, 2) => Op::Remove { word },
            _ => {
                return Err(Error::Cbor(
                    "snapshot operation kind does not match its width",
                ))
            }
        };
        Ok(Some(op))
    }

    /// Body items still to be decoded.
    #[must_use]
    pub fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Uncompressed CBOR bytes consumed so far.
    #[must_use]
    pub fn bytes_read(&self) -> u64 {
        self.bytes
    }

    fn read_head(&mut self) -> Result<(u8, u64)> {
        if let Some(head) = self.pending.take() {
            return Ok(head);
        }
        let (major, argument) = read_head(&mut self.inner)?;
        self.bytes += head_len(argument);
        Ok((major, argument))
    }

    fn read_uint(&mut self) -> Result<u64> {
        let (major, value) = self.read_head()?;
        if major != 0 {
            return Err(Error::Cbor("snapshot field is not an unsigned integer"));
        }
        Ok(value)
    }

    fn read_text(&mut self) -> Result<String> {
        let (major, length) = self.read_head()?;
        if major != 3 {
            return Err(Error::Cbor("snapshot word is not text"));
        }
        self.read_text_body(length)
    }

    fn read_text_body(&mut self, length: u64) -> Result<String> {
        let length =
            usize::try_from(length).map_err(|_| Error::Cbor("snapshot word is too long"))?;
        let mut word = vec![0u8; length];
        self.inner.read_exact(&mut word)?;
        self.bytes += length as u64;
        String::from_utf8(word).map_err(|_| Error::Cbor("snapshot word is not UTF-8"))
    }

    fn read_digest(&mut self) -> Result<[u8; 32]> {
        let (major, length) = self.read_head()?;
        if major != 2 || length != 32 {
            return Err(Error::Cbor("snapshot digest is not a 32-byte string"));
        }
        let mut digest = [0u8; 32];
        self.inner.read_exact(&mut digest)?;
        self.bytes += 32;
        Ok(digest)
    }
}

impl<R: Read> PairSource for SnapshotReader<R> {
    fn next_pair(&mut self) -> Result<Option<(String, [u8; 32])>> {
        Self::next_pair(self)
    }

    fn bytes_read(&self) -> u64 {
        Self::bytes_read(self)
    }
}

/// Replays a delta over its parent chain as one sorted pair stream.
pub struct DeltaSource<'a> {
    delta: SnapshotReader<Box<dyn Read + 'a>>,
    parent: Box<dyn PairSource + 'a>,
    next_op: Option<Op>,
    next_parent: Option<(String, [u8; 32])>,
    last_word: Option<String>,
}

impl<'a> DeltaSource<'a> {
    /// Pair `delta` (an opened delta entry) with the already reconstructed
    /// `parent` index.
    pub fn new(
        mut delta: SnapshotReader<Box<dyn Read + 'a>>,
        mut parent: Box<dyn PairSource + 'a>,
    ) -> Result<Self> {
        if delta.header().kind != EntryKind::Delta {
            return Err(Error::Cbor("snapshot entry is full, not a delta"));
        }
        let next_op = delta.next_op()?;
        Ok(Self {
            next_op,
            next_parent: parent.next_pair()?,
            delta,
            parent,
            last_word: None,
        })
    }

    fn emit(&mut self, word: String, digest: [u8; 32]) -> Result<Option<(String, [u8; 32])>> {
        if let Some(last) = &self.last_word {
            if word.as_str() <= last.as_str() {
                return Err(Error::Inconsistent(format!(
                    "rebuilt commit index is not strictly sorted at {word}"
                )));
            }
        }
        self.last_word = Some(word.clone());
        Ok(Some((word, digest)))
    }
}

impl PairSource for DeltaSource<'_> {
    fn next_pair(&mut self) -> Result<Option<(String, [u8; 32])>> {
        loop {
            let order = match (self.next_op.as_ref(), self.next_parent.as_ref()) {
                (None, None) => return Ok(None),
                (None, Some(_)) => Ordering::Greater,
                (Some(op), None) => {
                    if matches!(op, Op::Add { .. }) {
                        Ordering::Less
                    } else {
                        return Err(missing_word(op));
                    }
                }
                (Some(op), Some((word, _))) => {
                    let ordering = op.word().cmp(word.as_str());
                    if ordering == Ordering::Less && !matches!(op, Op::Add { .. }) {
                        return Err(missing_word(op));
                    }
                    ordering
                }
            };
            match order {
                Ordering::Less => {
                    let op = self.next_op.take().expect("operation checked above");
                    self.next_op = self.delta.next_op()?;
                    if let Op::Add { word, digest } = op {
                        return self.emit(word, digest);
                    }
                    unreachable!("non-add operations were rejected above");
                }
                Ordering::Greater => {
                    let (word, digest) = self.next_parent.take().expect("pair checked above");
                    self.next_parent = self.parent.next_pair()?;
                    return self.emit(word, digest);
                }
                Ordering::Equal => {
                    let op = self.next_op.take().expect("operation checked above");
                    self.next_op = self.delta.next_op()?;
                    self.next_parent = self.parent.next_pair()?;
                    match op {
                        Op::Change { word, digest } => return self.emit(word, digest),
                        Op::Remove { .. } => {}
                        Op::Add { word, .. } => {
                            return Err(Error::Inconsistent(format!(
                                "commit index delta adds an existing word: {word}"
                            )));
                        }
                    }
                }
            }
        }
    }

    fn bytes_read(&self) -> u64 {
        self.delta.bytes_read() + self.parent.bytes_read()
    }
}

fn missing_word(op: &Op) -> Error {
    Error::Inconsistent(format!(
        "commit index delta touches a missing word: {}",
        op.word()
    ))
}

/// Read one CBOR head: major type plus argument.
fn read_head<R: Read>(reader: &mut R) -> Result<(u8, u64)> {
    let mut first = [0u8; 1];
    reader.read_exact(&mut first)?;
    let major = first[0] >> 5;
    let info = first[0] & 0x1f;
    let argument = match info {
        0..=23 => u64::from(info),
        24 => u64::from(read_byte(reader)?),
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
        _ => return Err(Error::Cbor("snapshot uses an indefinite length")),
    };
    Ok((major, argument))
}

fn read_byte<R: Read>(reader: &mut R) -> Result<u8> {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte)?;
    Ok(byte[0])
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
