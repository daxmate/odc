//! Synthetic MDX fixtures for ODC tests.
//!
//! Everything produced here is generated from caller-supplied strings; no
//! dictionary content ships with the repository. Version 2 files go through
//! the mdictlib builder, version 1 files are written by hand with the wire
//! layout documented in docs/mdx-compat.md.

use std::io::{self, Write};
use std::path::Path;

/// Engine version written into version 1 fixtures.
pub const ENGINE_VERSION_V1: &str = "1.2";
/// Engine version written by the version 2 builder.
pub const ENGINE_VERSION_V2: &str = "2.0";

/// Write a version 2 (MDict 2.0) MDX file.
///
/// # Errors
///
/// Propagates builder and I/O failures.
pub fn write_v2_mdx(path: &Path, entries: &[(&str, &str)]) -> io::Result<()> {
    let mut builder = mdictlib::MdxBuilder::new();
    for (key, body) in entries {
        builder
            .add_entry(key, body)
            .map_err(|error| io::Error::other(error.to_string()))?;
    }
    let file = std::fs::File::create(path)?;
    builder
        .finish(io::BufWriter::new(file))
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(())
}

/// Write an MDD resource file from caller-supplied `(path, payload)` pairs.
///
/// Resource names are written verbatim, including the backslashes and leading
/// slash real dictionaries use, so the importer's normalisation is exercised.
///
/// # Errors
///
/// Propagates builder and I/O failures.
pub fn write_mdd(path: &Path, resources: &[(&str, &[u8])]) -> io::Result<()> {
    let mut builder = mdictlib::MddBuilder::new();
    for (name, bytes) in resources {
        builder
            .add_resource(name, bytes)
            .map_err(|error| io::Error::other(error.to_string()))?;
    }
    let file = std::fs::File::create(path)?;
    builder
        .finish(io::BufWriter::new(file))
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(())
}

/// Write a version 1.2 MDX file with UTF-16LE keys and records.
///
/// The file uses one key block and one record block, zlib compression and no
/// encryption. Keys are sorted by their UTF-16LE byte order.
///
/// # Errors
///
/// Returns an error for I/O failures, for keys that do not fit the one-byte
/// summary length (255 UTF-16 units) or for entries that do not fit the
/// 32-bit fields of the format.
pub fn write_v1_mdx(path: &Path, entries: &[(&str, &str)]) -> io::Result<()> {
    let mut rows: Vec<(String, Vec<u8>, Vec<u8>)> = entries
        .iter()
        .map(|(key, body)| (key.to_string(), encode(key), encode(body)))
        .collect();
    rows.sort_by(|left, right| left.1.cmp(&right.1));

    let mut record_stream = Vec::new();
    let mut starts = Vec::with_capacity(rows.len());
    for (_, _, body) in &rows {
        starts.push(u32::try_from(record_stream.len()).map_err(|_| invalid("record stream"))?);
        record_stream.extend_from_slice(body);
    }

    let mut key_block = Vec::new();
    for (index, (_, key, _)) in rows.iter().enumerate() {
        key_block.extend_from_slice(&starts[index].to_be_bytes());
        key_block.extend_from_slice(key);
        key_block.extend_from_slice(&[0, 0]);
    }

    let key_comp = zlib_block(&key_block);
    let record_comp = zlib_block(&record_stream);

    let mut key_info = Vec::new();
    key_info.extend_from_slice(
        &u32::try_from(rows.len())
            .map_err(|_| invalid("row count"))?
            .to_be_bytes(),
    );
    push_summary(&mut key_info, rows.first().map_or("", |row| row.0.as_str()))?;
    push_summary(&mut key_info, rows.last().map_or("", |row| row.0.as_str()))?;
    key_info.extend_from_slice(
        &u32::try_from(key_comp.len())
            .map_err(|_| invalid("key block"))?
            .to_be_bytes(),
    );
    key_info.extend_from_slice(
        &u32::try_from(key_block.len())
            .map_err(|_| invalid("key block"))?
            .to_be_bytes(),
    );

    let xml = format!(
        "<Dictionary GeneratedByEngineVersion=\"{ENGINE_VERSION_V1}\" RequiredEngineVersion=\"{ENGINE_VERSION_V1}\" Encoding=\"UTF-16\" Format=\"Html\" KeyCaseSensitive=\"No\" Encrypted=\"No\" Description=\"ODC synthetic fixture\" Title=\"ODC fixture\"/>"
    );
    let mut out = Vec::new();
    let xml_bytes = encode(&xml);
    out.extend_from_slice(
        &u32::try_from(xml_bytes.len())
            .map_err(|_| invalid("header"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&xml_bytes);
    out.extend_from_slice(&adler32(&xml_bytes).to_le_bytes());

    // Keyword section: one block.
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(rows.len())
            .map_err(|_| invalid("entry count"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(
        &u32::try_from(key_info.len())
            .map_err(|_| invalid("key index"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(
        &u32::try_from(key_comp.len())
            .map_err(|_| invalid("key block"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&key_info);
    out.extend_from_slice(&key_comp);

    // Record section: one block.
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(rows.len())
            .map_err(|_| invalid("record count"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&8u32.to_be_bytes());
    out.extend_from_slice(
        &u32::try_from(record_comp.len())
            .map_err(|_| invalid("record block"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(
        &u32::try_from(record_comp.len())
            .map_err(|_| invalid("record block"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(
        &u32::try_from(record_stream.len())
            .map_err(|_| invalid("record block"))?
            .to_be_bytes(),
    );
    out.extend_from_slice(&record_comp);

    let mut file = std::fs::File::create(path)?;
    file.write_all(&out)?;
    Ok(())
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("fixture {what} out of range"),
    )
}

/// One-byte UTF-16 length (in code units) followed by the encoded key.
fn push_summary(out: &mut Vec<u8>, key: &str) -> io::Result<()> {
    let units = u8::try_from(key.encode_utf16().count()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "fixture key longer than 255 UTF-16 units",
        )
    })?;
    out.push(units);
    out.extend_from_slice(&encode(key));
    Ok(())
}

fn encode(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn zlib_block(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&adler32(data).to_be_bytes());
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(data).expect("in-memory zlib write");
    out.extend_from_slice(&encoder.finish().expect("in-memory zlib finish"));
    out
}

/// Adler-32 as used by MDict block checksums.
#[must_use]
pub fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for byte in bytes {
        a = (a + u32::from(*byte)) % MOD;
        b = (b + a) % MOD;
    }
    (b << 16) | a
}
