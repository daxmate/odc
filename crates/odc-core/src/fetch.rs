//! Deepen a shallow repository from another ODC repository.
//!
//! `odc pack --shallow` keeps a single commit and marks the file with
//! `meta.shallow = 1`. `deepen` walks the source's first-parent chain from
//! HEAD, copies the missing commit rows and their index snapshots, and brings
//! over the source's block store so the destination converges on what a full
//! export would have produced.
//!
//! The copy runs inside one transaction. A commit that does not hash to its own
//! contents, a snapshot whose entry count disagrees with its commit, or a block
//! whose payload does not hash to its key aborts the whole fetch: the caller
//! never sees a half-written history.
//!
//! Nothing outside the commit graph is touched: `entries`, `staging`, `refs`,
//! `resources`, HEAD and the working set stay exactly as they were.

use crate::error::{Error, Result};
use crate::hash;
use crate::index;
use crate::pack::SHALLOW_META_KEY;
use crate::repo::{self, CommitRecord, Repo};
use crate::snapshot::EntryKind;
use crate::store;
use rusqlite::{params, Connection};
use std::path::Path;

/// Outcome of `odc fetch --depth`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchReport {
    /// Depth the caller asked for.
    pub depth_requested: u32,
    /// Commit rows written to the destination.
    pub commits_added: usize,
    /// Index snapshots written to the destination.
    pub commit_index_added: usize,
    /// Blocks copied from the source.
    pub blocks_added: usize,
    /// True when the chain now reaches genesis and `meta.shallow` is gone.
    pub complete: bool,
}

/// Extend `repo` down to `depth` commits by copying history out of `source`.
///
/// `depth` counts the commits visible from HEAD along the first-parent chain,
/// HEAD included. Copies are offline: `source` is another ODC repository, either
/// a directory or a packed `.odc` file.
pub fn deepen(repo: &mut Repo, source: &Path, depth: u32) -> Result<FetchReport> {
    if depth == 0 {
        return Err(Error::InvalidArgument(
            "fetch depth must be at least 1".to_string(),
        ));
    }
    if !repo.is_shallow()? {
        return Err(Error::InvalidArgument(
            "fetch requires a shallow repository (meta.shallow)".to_string(),
        ));
    }
    let head = repo
        .head()?
        .ok_or_else(|| Error::InvalidArgument("fetch requires at least one commit".to_string()))?;
    let src = Repo::open(source)?;
    if same_database(repo, &src) {
        return Err(Error::InvalidArgument(
            "source and destination are the same repository".to_string(),
        ));
    }

    let (chain, complete) = collect_chain(&src, head, depth)?;
    let pending = split_pending(repo, &chain)?;

    repo.connection.execute(
        "ATTACH DATABASE ?1 AS src",
        params![src.database_path().to_string_lossy().as_ref()],
    )?;
    let copied = copy_history(&mut repo.connection, &src, &pending, complete);
    let detached = repo.connection.execute("DETACH DATABASE src", []);

    let (commits_added, commit_index_added, blocks_added) = copied?;
    detached?;

    Ok(FetchReport {
        depth_requested: depth,
        commits_added,
        commit_index_added,
        blocks_added,
        complete,
    })
}

/// Walk the source's first-parent chain from `head`, collecting at most
/// `depth` commits. The flag is true when the walk reached genesis.
fn collect_chain(source: &Repo, head: [u8; 32], depth: u32) -> Result<(Vec<CommitRecord>, bool)> {
    let mut chain: Vec<CommitRecord> = Vec::new();
    let mut cursor = Some(head);
    while chain.len() < depth as usize {
        let Some(current) = cursor else { break };
        let record = source.commit_record(&current)?.ok_or_else(|| {
            Error::Inconsistent(format!(
                "source is missing commit {}",
                hash::to_hex(&current)
            ))
        })?;
        let computed = hash::commit_hash(
            &record.parent,
            &record.parent2,
            &record.entry_root,
            &record.block_root,
            record.entry_count,
        );
        if computed != record.hash {
            return Err(Error::Inconsistent(format!(
                "source commit {} does not match its contents",
                hash::to_hex(&record.hash)
            )));
        }
        cursor = if record.parent == hash::ZERO_HASH {
            None
        } else {
            Some(record.parent)
        };
        chain.push(record);
    }
    Ok((chain, cursor.is_none()))
}

/// Split the chain into the commits the destination already has and the ones it
/// is missing, checking that the shared prefix really is the same history.
fn split_pending<'a>(repo: &Repo, chain: &'a [CommitRecord]) -> Result<Vec<&'a CommitRecord>> {
    let mut pending = Vec::new();
    for record in chain {
        match repo.commit_record(&record.hash)? {
            Some(local) if local == *record => {}
            Some(_) => {
                return Err(Error::Inconsistent(format!(
                    "destination commit {} differs from the source",
                    hash::to_hex(&record.hash)
                )))
            }
            None => pending.push(record),
        }
    }
    Ok(pending)
}

/// Copy the pending commits, their snapshots and the source block store.
fn copy_history(
    destination: &mut Connection,
    source: &Repo,
    pending: &[&CommitRecord],
    complete: bool,
) -> Result<(usize, usize, usize)> {
    let transaction = destination.transaction()?;
    let mut commits_added = 0usize;
    let mut commit_index_added = 0usize;
    {
        let mut insert_commit = transaction.prepare(
            "INSERT INTO main.commits(commit_hash, parent, parent2, entry_root, block_root,
                                      entry_count, ts, author, message)
             SELECT commit_hash, parent, parent2, entry_root, block_root,
                    entry_count, ts, author, message
             FROM src.commits WHERE commit_hash = ?1",
        )?;
        let mut insert_index = transaction.prepare(
            "INSERT INTO main.commit_index(commit_hash, codec, raw_size, data)
             SELECT commit_hash, codec, raw_size, data
             FROM src.commit_index WHERE commit_hash = ?1",
        )?;
        let boundary = pending.len().checked_sub(1);
        for (position, record) in pending.iter().enumerate() {
            insert_commit.execute([record.hash.as_slice()])?;
            commits_added += 1;
            let header =
                index::index_header(&source.connection, &record.hash)?.ok_or_else(|| {
                    Error::Inconsistent(format!(
                        "source is missing the commit index for {}",
                        hash::to_hex(&record.hash)
                    ))
                })?;
            if header.entry_count != record.entry_count {
                return Err(Error::Inconsistent(format!(
                    "source index for {} covers {} entries, commit says {}",
                    hash::to_hex(&record.hash),
                    header.entry_count,
                    record.entry_count
                )));
            }
            // The oldest copied commit is the new boundary: its parent is not
            // copied, so a delta snapshot would be unreadable here.
            if Some(position) == boundary && header.kind == EntryKind::Delta {
                let rebuilt = source
                    .materialized_head_index(&record.hash)?
                    .ok_or_else(|| {
                        Error::Inconsistent(format!(
                            "source index for {} could not be rebuilt",
                            hash::to_hex(&record.hash)
                        ))
                    })?;
                transaction.execute(
                    "INSERT INTO main.commit_index(commit_hash, codec, raw_size, data)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        record.hash.as_slice(),
                        store::codec::ZSTD,
                        i64::try_from(rebuilt.0).unwrap_or(i64::MAX),
                        rebuilt.1
                    ],
                )?;
            } else if insert_index.execute([record.hash.as_slice()])? == 0 {
                return Err(Error::Inconsistent(format!(
                    "source is missing the commit index for {}",
                    hash::to_hex(&record.hash)
                )));
            }
            commit_index_added += 1;
        }
    }
    let blocks_added = copy_blocks(&transaction, &source.connection)?;
    if complete {
        store::remove_meta(&transaction, SHALLOW_META_KEY)?;
    }
    transaction.commit()?;
    Ok((commits_added, commit_index_added, blocks_added))
}

/// Copy every block the destination lacks, folding each payload hash back out of
/// the bytes before they land.
fn copy_blocks(destination: &rusqlite::Transaction<'_>, source: &Connection) -> Result<usize> {
    let mut added = 0usize;
    let mut exists = destination.prepare("SELECT 1 FROM main.blocks WHERE block_hash = ?1")?;
    let mut insert = destination.prepare(
        "INSERT INTO main.blocks(block_hash, codec, raw_size, data) VALUES (?1, ?2, ?3, ?4)",
    )?;
    let mut select =
        source.prepare("SELECT block_hash, codec, raw_size, data FROM blocks ORDER BY id")?;
    let mut rows = select.query([])?;
    while let Some(row) = rows.next()? {
        let block_hash: Vec<u8> = row.get(0)?;
        let codec_id: i64 = row.get(1)?;
        let raw_size: i64 = row.get(2)?;
        let data: Vec<u8> = row.get(3)?;
        if exists.exists([block_hash.as_slice()])? {
            continue;
        }
        let expected = usize::try_from(raw_size)
            .map_err(|_| Error::Inconsistent("negative block size".to_string()))?;
        let raw = repo::decode_block_data(codec_id, expected, &data)?;
        let stored = repo::hash_from_blob(&block_hash)?;
        if raw.len() != expected || hash::blake3(&raw) != stored {
            return Err(Error::Inconsistent(format!(
                "source block {} does not match its payload",
                hash::to_hex(&stored)
            )));
        }
        insert.execute(params![block_hash, codec_id, raw_size, data])?;
        added += 1;
    }
    Ok(added)
}

fn same_database(repo: &Repo, source: &Repo) -> bool {
    match (
        repo.database_path().canonicalize(),
        source.database_path().canonicalize(),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
