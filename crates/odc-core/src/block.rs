//! Raw block encoding and decoding (spec section 3).

use crate::error::{Error, Result};
use crate::varint;

/// Block magic, four ASCII bytes.
pub const MAGIC: [u8; 4] = *b"ODB1";

/// Bit 0 of the flags byte marks a trailing offset index.
pub const FLAG_OFFSET_INDEX: u8 = 0x01;

/// One `(key, payload)` record inside a block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Entry key, UTF-8 bytes of the normalised word.
    pub key: Vec<u8>,
    /// Canonical CBOR payload bytes.
    pub payload: Vec<u8>,
}

impl Record {
    /// Build a record from a key and payload.
    #[must_use]
    pub fn new(key: impl Into<Vec<u8>>, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            key: key.into(),
            payload: payload.into(),
        }
    }

    /// Number of bytes this record occupies inside a block.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        varint::encoded_len(self.key.len() as u64)
            + self.key.len()
            + varint::encoded_len(self.payload.len() as u64)
            + self.payload.len()
    }

    fn write(&self, out: &mut Vec<u8>) {
        varint::encode(self.key.len() as u64, out);
        out.extend_from_slice(&self.key);
        varint::encode(self.payload.len() as u64, out);
        out.extend_from_slice(&self.payload);
    }
}

/// A decoded block together with its offset index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedBlock {
    /// Records in stored order (strictly increasing key bytes).
    pub records: Vec<Record>,
    /// Byte offset of every record from the start of the raw block.
    pub offsets: Vec<u64>,
    /// Whether the producer included the offset index.
    pub has_index: bool,
}

fn check_order(records: &[Record]) -> Result<()> {
    let mut previous: Option<&[u8]> = None;
    for record in records {
        if record.key.is_empty() {
            return Err(Error::InvalidArgument("empty record key".to_string()));
        }
        if std::str::from_utf8(&record.key).is_err() {
            return Err(Error::InvalidArgument(
                "record key is not UTF-8".to_string(),
            ));
        }
        if let Some(previous) = previous {
            if record.key.as_slice() < previous {
                return Err(Error::UnsortedRecords);
            }
            if record.key.as_slice() == previous {
                return Err(Error::DuplicateKey(
                    String::from_utf8_lossy(&record.key).into_owned(),
                ));
            }
        }
        previous = Some(record.key.as_slice());
    }
    Ok(())
}

/// Encode a block with the canonical layout: magic, flags with the offset
/// index bit set, record count, records, then one varint offset per record.
pub fn encode(records: &[Record]) -> Result<Vec<u8>> {
    check_order(records)?;
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.push(FLAG_OFFSET_INDEX);
    varint::encode(records.len() as u64, &mut out);
    let mut offsets = Vec::with_capacity(records.len());
    for record in records {
        offsets.push(out.len() as u64);
        record.write(&mut out);
    }
    for offset in offsets {
        varint::encode(offset, &mut out);
    }
    Ok(out)
}

/// Decode a block produced by the canonical encoder (or any conforming one).
///
/// The offset index is optional on read: producers that clear bit 0 may omit
/// it. When present the offsets must match the actual record positions.
pub fn decode(raw: &[u8]) -> Result<DecodedBlock> {
    if raw.len() < 6 {
        return Err(Error::Truncated("block header"));
    }
    if raw[0..4] != MAGIC {
        return Err(Error::BadMagic(format!(
            "expected ODB1, found {:?}",
            &raw[0..4]
        )));
    }
    let flags = raw[4];
    if flags & !FLAG_OFFSET_INDEX != 0 {
        return Err(Error::BadMagic(format!(
            "reserved flag bits set: {flags:#04x}"
        )));
    }
    let (count, used) = varint::decode(&raw[5..])?;
    let mut position = 5 + used;
    let count = usize::try_from(count).map_err(|_| Error::Cbor("record count out of range"))?;
    if count > raw.len() {
        return Err(Error::Truncated("block records"));
    }
    let mut records = Vec::with_capacity(count.min(4096));
    let mut offsets = Vec::with_capacity(count.min(4096));
    for _ in 0..count {
        offsets.push(position as u64);
        let (key_len, used) = varint::decode(&raw[position..])?;
        position += used;
        let key_len = usize::try_from(key_len).map_err(|_| Error::Truncated("record key"))?;
        let key_end = position
            .checked_add(key_len)
            .ok_or(Error::Truncated("record key"))?;
        let key = raw
            .get(position..key_end)
            .ok_or(Error::Truncated("record key"))?
            .to_vec();
        position = key_end;
        let (payload_len, used) = varint::decode(&raw[position..])?;
        position += used;
        let payload_len =
            usize::try_from(payload_len).map_err(|_| Error::Truncated("record payload"))?;
        let payload_end = position
            .checked_add(payload_len)
            .ok_or(Error::Truncated("record payload"))?;
        let payload = raw
            .get(position..payload_end)
            .ok_or(Error::Truncated("record payload"))?
            .to_vec();
        position = payload_end;
        records.push(Record { key, payload });
    }
    check_order(&records)?;
    let has_index = flags & FLAG_OFFSET_INDEX != 0;
    if has_index {
        for expected in &offsets {
            let (stored, used) = varint::decode(&raw[position..])?;
            position += used;
            if stored != *expected {
                return Err(Error::Inconsistent(format!(
                    "offset index entry {stored} does not match record position {expected}"
                )));
            }
        }
    }
    if position != raw.len() {
        return Err(Error::TrailingData("block"));
    }
    Ok(DecodedBlock {
        records,
        offsets,
        has_index,
    })
}

/// Decode exactly one record (used to read a single entry from its slice).
pub fn decode_record(bytes: &[u8]) -> Result<Record> {
    let (key_len, used) = varint::decode(bytes)?;
    let mut position = used;
    let key_len = usize::try_from(key_len).map_err(|_| Error::Truncated("record key"))?;
    let key_end = position
        .checked_add(key_len)
        .ok_or(Error::Truncated("record key"))?;
    let key = bytes
        .get(position..key_end)
        .ok_or(Error::Truncated("record key"))?
        .to_vec();
    position = key_end;
    let (payload_len, used) = varint::decode(&bytes[position..])?;
    position += used;
    let payload_len =
        usize::try_from(payload_len).map_err(|_| Error::Truncated("record payload"))?;
    let payload_end = position
        .checked_add(payload_len)
        .ok_or(Error::Truncated("record payload"))?;
    let payload = bytes
        .get(position..payload_end)
        .ok_or(Error::Truncated("record payload"))?
        .to_vec();
    if payload_end != bytes.len() {
        return Err(Error::TrailingData("record"));
    }
    Ok(Record { key, payload })
}
