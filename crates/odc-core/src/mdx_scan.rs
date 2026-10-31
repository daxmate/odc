//! Tolerant scanning and repair of the MDX 2.0 record section.
//!
//! The `mdictlib` parser validates every record block it reads and aborts the
//! whole import on the first corrupt block. Real dictionaries occasionally
//! ship with a single damaged block (see `docs/mdx-compat.md`), so ODC offers a
//! repair path instead: scan the record section up front, and when a block is
//! corrupt write a temporary copy in which that block is replaced by a sentinel
//! payload of exactly its declared decompressed size.
//!
//! Sentinel payloads keep every decompressed record offset unchanged, so the
//! key section and all record boundaries still line up. The importer then drops
//! the sentinel entries and reports how many entries were skipped.
//!
//! Only version 2 files can be repaired. Version 1.2 files keep the stricter
//! behaviour documented in `docs/mdx-compat.md`.

use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::{Error, Result};

/// Prefix of the sentinel payload written into repaired record blocks.
///
/// Entries whose HTML contains this marker were fabricated by
/// `write_repaired_copy` and must be dropped by the importer.
pub const SENTINEL_PREFIX: &str = "ODC-SKIPPED-BLOCK-";

/// Size of the 4-byte checksum that follows a version 2 header string.
const HEADER_CHECKSUM_BYTES: u64 = 4;
/// Version 2 keyword section header: five `u64` fields plus its checksum.
const KEY_SECTION_HEADER_BYTES: u64 = 44;
/// Version 2 record section header: four `u64` fields.
const RECORD_HEADER_BYTES: u64 = 32;
/// One record block index entry: compressed size, decompressed size.
const RECORD_INDEX_ENTRY_BYTES: u64 = 16;
/// `comp_type` value for zlib-compressed record blocks.
const ZLIB_COMP_TYPE: u32 = 2;
/// Upper bound accepted for one record block, compressed or decompressed.
const MAX_BLOCK_BYTES: u64 = 256 * 1024 * 1024;
/// Read buffer used while inflating a record block.
const INFLATE_CHUNK_BYTES: usize = 64 * 1024;

static COPY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One record block as described by the record section index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordBlock {
    /// Zero-based position in the record section index.
    pub ordinal: u64,
    /// Absolute offset of the block (`comp_type` + checksum + payload).
    pub file_offset: u64,
    /// Compressed size, including the 8-byte block header.
    pub compressed_size: u64,
    /// Declared decompressed size.
    pub decompressed_size: u64,
    /// Offset of this block's first byte in the decompressed record stream.
    pub decompressed_offset: u64,
}

/// One record block that failed validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorruptBlock {
    /// Zero-based position in the record section index.
    pub ordinal: u64,
    /// Absolute offset of the block in the original file.
    pub file_offset: u64,
    /// Compressed size, including the 8-byte block header.
    pub compressed_size: u64,
    /// Declared decompressed size.
    pub declared_size: u64,
    /// Bytes actually produced before the decoder failed, when known.
    pub decompressed_size: Option<u64>,
    /// Offset of the block in the decompressed record stream.
    pub decompressed_offset: u64,
    /// Human-readable reason the block was rejected.
    pub detail: String,
}

impl CorruptBlock {
    /// Half-open range of decompressed record offsets covered by the block.
    ///
    /// The end is always the declared size, even when the decoder produced
    /// fewer bytes, because that is the range the key section addresses.
    #[must_use]
    pub fn decompressed_range(&self) -> (u64, u64) {
        (
            self.decompressed_offset,
            self.decompressed_offset.saturating_add(self.declared_size),
        )
    }
}

/// Parsed record section of a version 2 MDX file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordSection {
    /// Absolute offset of the 32-byte record section header.
    pub offset: u64,
    /// Number of record blocks declared by the header.
    pub num_blocks: u64,
    /// Number of entries declared by the header.
    pub num_entries: u64,
    /// Size of the compressed block index.
    pub index_len: u64,
    /// Total size of the compressed record blocks.
    pub blocks_len: u64,
    /// Every record block, in file order.
    pub blocks: Vec<RecordBlock>,
    /// Blocks that failed validation, in file order.
    pub corrupt: Vec<CorruptBlock>,
}

impl RecordSection {
    /// Whether the record section contains at least one corrupt block.
    #[must_use]
    pub fn has_corrupt_blocks(&self) -> bool {
        !self.corrupt.is_empty()
    }
}

/// A temporary file that deletes itself when dropped.
#[derive(Debug)]
pub struct TemporaryCopy {
    path: PathBuf,
}

impl TemporaryCopy {
    /// Path of the temporary file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Locate and verify the record section of a version 2 MDX file.
///
/// Returns `Ok(None)` when the file is not a version 2 MDX file (version 1.2
/// keeps the strict import path) or when its header cannot be parsed at all;
/// `mdictlib` then reports the failure as it always did.
///
/// # Errors
///
/// * `Error::Mdx` when the file claims to be version 2 but its record
///   section does not match the documented layout (the offset self-check
///   failed), or when a record block is structurally impossible.
/// * `Error::Io` when the file cannot be read.
pub fn scan(path: &Path) -> Result<Option<RecordSection>> {
    let mut reader = BufReader::with_capacity(64 * 1024, File::open(path)?);
    let file_size = reader.get_ref().metadata()?.len();
    let Some(offset) = locate_record_section(&mut reader, file_size)? else {
        return Ok(None);
    };

    let mut header = [0_u8; 32];
    reader.seek(SeekFrom::Start(offset))?;
    reader.read_exact(&mut header).map_err(|error| {
        layout_error(format!(
            "record section header at {offset} is unreadable: {error}"
        ))
    })?;
    let num_blocks = be_u64(&header[0..8]);
    let num_entries = be_u64(&header[8..16]);
    let index_len = be_u64(&header[16..24]);
    let blocks_len = be_u64(&header[24..32]);
    if !index_len.is_multiple_of(RECORD_INDEX_ENTRY_BYTES) {
        return Err(layout_error(format!(
            "record index length {index_len} is not a multiple of {RECORD_INDEX_ENTRY_BYTES}"
        )));
    }
    if num_blocks != index_len / RECORD_INDEX_ENTRY_BYTES {
        return Err(layout_error(format!(
            "record header declares {num_blocks} blocks but the index holds {}",
            index_len / RECORD_INDEX_ENTRY_BYTES
        )));
    }
    // Every byte after the record section header belongs to the index or to the
    // blocks; if that does not add up the layout is not one we understand and
    // guessing offsets would corrupt the repaired copy.
    let expected_size = offset
        .checked_add(RECORD_HEADER_BYTES)
        .and_then(|value| value.checked_add(index_len))
        .and_then(|value| value.checked_add(blocks_len))
        .ok_or_else(|| layout_error("record section size overflows u64".to_string()))?;
    if expected_size != file_size {
        return Err(layout_error(format!(
            "record section self-check failed: {RECORD_HEADER_BYTES} + {index_len} + {blocks_len} \
             bytes after offset {offset} describe {expected_size} bytes but the file holds \
             {file_size}"
        )));
    }

    let index_len_usize = usize::try_from(index_len)
        .map_err(|_| layout_error(format!("record index length {index_len} exceeds usize")))?;
    let mut index = vec![0_u8; index_len_usize];
    reader.read_exact(&mut index)?;

    let mut blocks = Vec::with_capacity(usize::try_from(num_blocks).unwrap_or(0));
    let mut corrupt = Vec::new();
    let mut file_offset = offset + RECORD_HEADER_BYTES + index_len;
    let mut decompressed_offset = 0_u64;
    for (position, entry) in index.as_chunks::<16>().0.iter().enumerate() {
        let compressed_size = be_u64(&entry[0..8]);
        let decompressed_size = be_u64(&entry[8..16]);
        let ordinal = u64::try_from(position)
            .map_err(|_| layout_error("record block ordinal exceeds u64".to_string()))?;
        let block = RecordBlock {
            ordinal,
            file_offset,
            compressed_size,
            decompressed_size,
            decompressed_offset,
        };
        verify_block(&mut reader, &block, &mut corrupt)?;
        blocks.push(block);
        file_offset = file_offset
            .checked_add(compressed_size)
            .ok_or_else(|| layout_error("record block offset overflows u64".to_string()))?;
        decompressed_offset = decompressed_offset
            .checked_add(decompressed_size)
            .ok_or_else(|| layout_error("record block size overflows u64".to_string()))?;
    }

    Ok(Some(RecordSection {
        offset,
        num_blocks,
        num_entries,
        index_len,
        blocks_len,
        blocks,
        corrupt,
    }))
}

/// Write a repaired copy of `path` in the system temporary directory.
///
/// The copy repeats every byte before the record section, replaces each corrupt
/// block with a sentinel payload that decompresses to exactly the declared
/// size, and rewrites the record header and index. The returned
/// `TemporaryCopy` removes the file when dropped.
///
/// # Errors
///
/// * `Error::InvalidArgument` when `section` has no corrupt blocks (a
///   repair would be pointless; callers should import the original file).
/// * `Error::Io` when the file cannot be read or the temporary copy cannot
///   be written.
pub fn write_repaired_copy(path: &Path, section: &RecordSection) -> Result<TemporaryCopy> {
    if section.corrupt.is_empty() {
        return Err(Error::InvalidArgument(
            "refusing to write a repaired copy without corrupt blocks".to_string(),
        ));
    }
    let corrupt_at: HashMap<u64, usize> = section
        .corrupt
        .iter()
        .enumerate()
        .map(|(index, block)| (block.ordinal, index))
        .collect();

    // Compress the sentinels first: their size decides the new index.
    let mut sentinels: Vec<Option<Vec<u8>>> = vec![None; section.blocks.len()];
    let mut blocks_len = 0_u64;
    for (position, block) in section.blocks.iter().enumerate() {
        let compressed_size = match corrupt_at.get(&block.ordinal) {
            Some(_) => {
                let sentinel = sentinel_block(block.ordinal, block.decompressed_size)?;
                let size = u64::try_from(sentinel.len())
                    .map_err(|_| layout_error("sentinel block exceeds u64".to_string()))?;
                sentinels[position] = Some(sentinel);
                size
            }
            None => block.compressed_size,
        };
        blocks_len = blocks_len
            .checked_add(compressed_size)
            .ok_or_else(|| layout_error("repaired block sizes overflow u64".to_string()))?;
    }

    let sequence = COPY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "odc-mdx-repair-{}-{sequence}.mdx",
        std::process::id()
    ));
    let copy = TemporaryCopy {
        path: target.clone(),
    };

    let mut source = BufReader::with_capacity(1 << 16, File::open(path)?);
    let mut writer = BufWriter::with_capacity(1 << 20, File::create(&target)?);
    std::io::copy(&mut source.by_ref().take(section.offset), &mut writer)?;

    writer.write_all(&section.num_blocks.to_be_bytes())?;
    writer.write_all(&section.num_entries.to_be_bytes())?;
    writer.write_all(&section.index_len.to_be_bytes())?;
    writer.write_all(&blocks_len.to_be_bytes())?;

    for (position, block) in section.blocks.iter().enumerate() {
        let compressed_size = match &sentinels[position] {
            Some(sentinel) => u64::try_from(sentinel.len())
                .map_err(|_| layout_error("sentinel block exceeds u64".to_string()))?,
            None => block.compressed_size,
        };
        writer.write_all(&compressed_size.to_be_bytes())?;
        writer.write_all(&block.decompressed_size.to_be_bytes())?;
    }

    let blocks_start = section
        .offset
        .checked_add(RECORD_HEADER_BYTES)
        .and_then(|value| value.checked_add(section.index_len))
        .ok_or_else(|| layout_error("record block offset overflows u64".to_string()))?;
    source.seek(SeekFrom::Start(blocks_start))?;
    for (position, block) in section.blocks.iter().enumerate() {
        match &sentinels[position] {
            Some(sentinel) => {
                writer.write_all(sentinel)?;
                // The damaged block is replaced, but the source cursor still has
                // to step over its original bytes.
                source.seek(SeekFrom::Current(
                    i64::try_from(block.compressed_size)
                        .map_err(|_| layout_error("record block offset exceeds i64".to_string()))?,
                ))?;
            }
            None => {
                std::io::copy(
                    &mut source.by_ref().take(block.compressed_size),
                    &mut writer,
                )?;
            }
        }
    }
    writer.flush()?;
    drop(writer);
    Ok(copy)
}

/// Whether an HTML payload was produced by `write_repaired_copy`.
#[must_use]
pub fn is_sentinel_payload(html: &str) -> bool {
    html.contains(SENTINEL_PREFIX)
}

/// Plain-text sentinel repeated until it fills `size` bytes.
fn sentinel_plaintext(ordinal: u64, size: u64) -> Result<Vec<u8>> {
    let size = usize::try_from(size)
        .map_err(|_| layout_error(format!("record block {ordinal} exceeds usize")))?;
    let marker = format!("{SENTINEL_PREFIX}{ordinal}");
    let mut out = Vec::with_capacity(size);
    while out.len() < size {
        out.extend_from_slice(marker.as_bytes());
    }
    out.truncate(size);
    Ok(out)
}

/// Build the replacement block: `comp_type`, checksum, then a zlib payload.
fn sentinel_block(ordinal: u64, size: u64) -> Result<Vec<u8>> {
    let plaintext = sentinel_plaintext(ordinal, size)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&plaintext)?;
    let compressed = encoder.finish()?;
    let mut block = Vec::with_capacity(8 + compressed.len());
    block.extend_from_slice(&ZLIB_COMP_TYPE.to_le_bytes());
    block.extend_from_slice(&adler32(&plaintext).to_be_bytes());
    block.extend_from_slice(&compressed);
    Ok(block)
}

/// Resolve the absolute offset of the record section, or `None` for version 1.
fn locate_record_section(reader: &mut BufReader<File>, file_size: u64) -> Result<Option<u64>> {
    let mut length = [0_u8; 4];
    if reader.read_exact(&mut length).is_err() {
        return Ok(None);
    }
    let header_len = u64::from(u32::from_be_bytes(length));
    if header_len == 0 || header_len > file_size.saturating_sub(4) {
        return Ok(None);
    }
    let header_len_usize =
        usize::try_from(header_len).map_err(|_| layout_error("header too large".to_string()))?;
    let mut header = vec![0_u8; header_len_usize];
    if reader.read_exact(&mut header).is_err() {
        return Ok(None);
    }
    let Some(version) = engine_version(&header) else {
        return Ok(None);
    };
    if !version.starts_with('2') {
        return Ok(None);
    }

    let mut key_header = [0_u8; 40];
    let key_header_offset = 4 + header_len + HEADER_CHECKSUM_BYTES;
    reader.seek(SeekFrom::Start(key_header_offset))?;
    if reader.read_exact(&mut key_header).is_err() {
        return Err(layout_error(
            "version 2 header is truncated before the keyword section".to_string(),
        ));
    }
    let index_comp = be_u64(&key_header[24..32]);
    let blocks_len = be_u64(&key_header[32..40]);
    let offset = key_header_offset
        .checked_add(KEY_SECTION_HEADER_BYTES)
        .and_then(|value| value.checked_add(index_comp))
        .and_then(|value| value.checked_add(blocks_len))
        .ok_or_else(|| layout_error("record section offset overflows u64".to_string()))?;
    if offset >= file_size {
        return Err(layout_error(format!(
            "record section offset {offset} lies beyond the end of the file ({file_size})"
        )));
    }
    Ok(Some(offset))
}

/// `GeneratedByEngineVersion` of a version 2 header, if it can be read.
fn engine_version(header: &[u8]) -> Option<String> {
    let xml = if header.len() >= 2 && header[0] == b'<' && header[1] == 0 {
        let units: Vec<u16> = header
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes(*pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(header).into_owned()
    };
    let marker = "GeneratedByEngineVersion=\"";
    let start = xml.find(marker)? + marker.len();
    let rest = &xml[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Validate one record block and record it when it is corrupt.
fn verify_block(
    reader: &mut BufReader<File>,
    block: &RecordBlock,
    corrupt: &mut Vec<CorruptBlock>,
) -> Result<()> {
    if block.compressed_size > MAX_BLOCK_BYTES || block.decompressed_size > MAX_BLOCK_BYTES {
        return Err(layout_error(format!(
            "record block {} declares {} compressed / {} decompressed bytes, above the {} byte limit",
            block.ordinal, block.compressed_size, block.decompressed_size, MAX_BLOCK_BYTES
        )));
    }
    let compressed_size = usize::try_from(block.compressed_size)
        .map_err(|_| layout_error("record block exceeds usize".to_string()))?;
    let mut raw = vec![0_u8; compressed_size];
    reader.read_exact(&mut raw)?;
    if raw.len() < 8 {
        corrupt.push(CorruptBlock {
            ordinal: block.ordinal,
            file_offset: block.file_offset,
            compressed_size: block.compressed_size,
            declared_size: block.decompressed_size,
            decompressed_size: None,
            decompressed_offset: block.decompressed_offset,
            detail: format!("block holds {} bytes, below its 8-byte header", raw.len()),
        });
        return Ok(());
    }
    let comp_type = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    let expected_adler = u32::from_be_bytes([raw[4], raw[5], raw[6], raw[7]]);
    let payload = &raw[8..];
    if comp_type != ZLIB_COMP_TYPE {
        // Unknown codecs are only checked structurally: ODC never guesses how
        // to decompress a block it does not know.
        if payload.is_empty() {
            corrupt.push(CorruptBlock {
                ordinal: block.ordinal,
                file_offset: block.file_offset,
                compressed_size: block.compressed_size,
                declared_size: block.decompressed_size,
                decompressed_size: Some(0),
                decompressed_offset: block.decompressed_offset,
                detail: format!("comp_type {comp_type} block carries no payload"),
            });
        }
        return Ok(());
    }

    let declared = usize::try_from(block.decompressed_size)
        .map_err(|_| layout_error("record block exceeds usize".to_string()))?;
    let (data, failure) = inflate(payload, declared);
    let checksum = adler32(&data);
    let detail = match failure {
        Some(reason) => Some(reason),
        None if data.len() != declared => Some(format!(
            "decompressed to {} of {} declared bytes",
            data.len(),
            declared
        )),
        None if checksum != expected_adler => Some(format!(
            "adler32 mismatch (declared {expected_adler:08x}, computed {checksum:08x})"
        )),
        None => None,
    };
    if let Some(detail) = detail {
        corrupt.push(CorruptBlock {
            ordinal: block.ordinal,
            file_offset: block.file_offset,
            compressed_size: block.compressed_size,
            declared_size: block.decompressed_size,
            decompressed_size: u64::try_from(data.len()).ok(),
            decompressed_offset: block.decompressed_offset,
            detail,
        });
    }
    Ok(())
}

/// Inflate a zlib payload, keeping whatever the decoder produced on failure.
fn inflate(payload: &[u8], declared: usize) -> (Vec<u8>, Option<String>) {
    let mut out = Vec::with_capacity(declared.min(1 << 20));
    let mut decoder = ZlibDecoder::new(payload);
    let mut chunk = vec![0_u8; INFLATE_CHUNK_BYTES];
    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => return (out, None),
            Ok(read) => {
                out.extend_from_slice(&chunk[..read]);
                if out.len() > declared {
                    return (
                        out,
                        Some(format!(
                            "decompressed payload exceeds the declared {declared} bytes"
                        )),
                    );
                }
            }
            Err(error) => return (out, Some(format!("zlib: {error}"))),
        }
    }
}

/// Adler-32 checksum, as used by the zlib stream wrapper.
fn adler32(bytes: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    let mut low = 1_u32;
    let mut high = 0_u32;
    for chunk in bytes.chunks(5_552) {
        for &byte in chunk {
            low += u32::from(byte);
            high += low;
        }
        low %= MODULUS;
        high %= MODULUS;
    }
    (high << 16) | low
}

fn be_u64(bytes: &[u8]) -> u64 {
    let mut value = [0_u8; 8];
    value.copy_from_slice(bytes);
    u64::from_be_bytes(value)
}

fn layout_error(detail: String) -> Error {
    Error::Mdx(format!("record section: {detail}"))
}
