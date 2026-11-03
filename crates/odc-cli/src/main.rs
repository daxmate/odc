//! `odc` command line interface (M0).

use clap::{Parser, Subcommand};
use odc_core::cbor;
use odc_core::entry::{normalize_key, Def, Entry};
use odc_core::hash;
use odc_core::html::{HtmlImportSummary, HtmlMode, HtmlOptions};
use odc_core::link::LinkResolution;
use odc_core::merge::{self, Choice, MergeKind, MergeOutcome, Strategy};
use odc_core::patch;
use odc_core::repo::Repo;
use serde_json::json;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "odc",
    version,
    about = "Open Dictionary Container: content-addressed dictionary format",
    long_about = None
)]
struct Cli {
    /// Repository directory (contains odc.db).
    #[arg(long, global = true, default_value = ".")]
    repo: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Create a repository and its genesis commit.
    Init,
    /// Import entries into the working set.
    Import {
        /// Input file (JSONL, CSV or MDX).
        file: PathBuf,
        /// Input format.
        #[arg(long, value_parser = ["jsonl", "csv", "mdx"], default_value = "jsonl")]
        format: String,
        /// Delete working-set entries that are absent from the input file.
        #[arg(long)]
        prune: bool,
        /// Stop after this many entries (MDX only).
        #[arg(short = 'n', long)]
        limit: Option<usize>,
        /// Print progress to stderr (MDX only).
        #[arg(long)]
        progress: bool,
        /// HTML handling for imported entry bodies (MDX only).
        #[arg(long, value_parser = ["format", "check", "off"], default_value = "format")]
        html: String,
    },
    /// Record the working set as a new commit.
    Commit {
        /// Commit message.
        #[arg(short, long)]
        message: String,
        /// Author recorded with the commit.
        #[arg(long, default_value = odc_core::DEFAULT_AUTHOR)]
        author: String,
        /// Override the commit timestamp (unix seconds); mostly for tests.
        #[arg(long)]
        timestamp: Option<i64>,
    },
    /// Show commit history, newest first.
    Log {
        /// Maximum number of commits.
        #[arg(short = 'n', long)]
        limit: Option<usize>,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Print one entry.
    Show {
        /// Word to look up (normalised automatically).
        word: String,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Look up one committed entry; exits with code 4 when it is missing.
    Lookup {
        /// Word to look up (normalised automatically).
        word: String,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// List committed entries whose normalised key starts with a prefix.
    Search {
        /// Key prefix (normalised automatically).
        #[arg(long)]
        prefix: String,
        /// Maximum number of entries.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Export every committed entry.
    Export {
        /// Output format.
        #[arg(long, value_parser = ["jsonl", "csv"], default_value = "jsonl")]
        format: String,
        /// Output file; stdout when omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Export the repository as one portable .odc file.
    Pack {
        /// Write only HEAD's commit, without its history.
        #[arg(long)]
        shallow: bool,
        /// Output file (default: `<repo>.odc` next to the repository).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Replace an existing output file.
        #[arg(long)]
        force: bool,
    },
    /// Compare two revisions (word level).
    Diff {
        /// Base revision (HEAD, HEAD~1 or a hash prefix).
        from: String,
        /// Target revision.
        to: String,
        /// Emit JSON.
        #[arg(long)]
        json: bool,
    },
    /// Merge another commit into HEAD.
    Merge {
        /// Our revision; must resolve to HEAD.
        ours: Option<String>,
        /// Their revision.
        theirs: Option<String>,
        /// Strategy: auto, ours, theirs or union.
        #[arg(long, default_value = "auto")]
        strategy: String,
        /// Finish an interrupted merge instead of starting a new one.
        #[arg(long = "continue")]
        continue_merge: bool,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Resolve one merge conflict.
    Resolve {
        /// Conflicting word.
        word: String,
        /// Keep our payload.
        #[arg(long)]
        ours: bool,
        /// Keep their payload.
        #[arg(long)]
        theirs: bool,
        /// Keep both payloads.
        #[arg(long)]
        union: bool,
        /// Take the payload from this canonical CBOR file.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// List recorded merge conflicts.
    Conflicts {
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Check every block, entry and commit.
    Verify,
    /// Delete blocks and commit indexes no reachable commit needs.
    Gc {
        /// Commits to keep on each first-parent chain (at least 1).
        #[arg(long, value_name = "N")]
        keep_last: usize,
        /// Report what would be freed without writing.
        #[arg(long)]
        dry_run: bool,
        /// Rebuild the database file afterwards.
        #[arg(long)]
        vacuum: bool,
    },
    /// Show the working set relative to HEAD.
    Status,
    /// Check entry HTML without writing anything.
    CheckHtml {
        /// MDX file to inspect (default: every entry of the repository).
        path: Option<PathBuf>,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
        /// Stop after this many entries (debugging aid).
        #[arg(short = 'n', long)]
        limit: Option<usize>,
    },
    /// Normalise entry HTML in the working set.
    Normalize {
        /// Write the result instead of only reporting it.
        #[arg(long)]
        apply: bool,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Patch creation and application (specified, implemented in M3).
    Patch {
        #[command(subcommand)]
        command: PatchCommand,
    },
}

#[derive(Subcommand, Debug)]
enum PatchCommand {
    /// Create a patch between two revisions.
    Create {
        /// Base revision the receiver must have checked out.
        base: String,
        /// Target revision the patch advances to.
        target: String,
        /// Where to write the patch file.
        #[arg(short, long)]
        output: PathBuf,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Apply a patch file.
    Apply {
        /// Patch file to apply.
        file: PathBuf,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Verify a patch file against this repository.
    Verify {
        /// Patch file to verify.
        file: PathBuf,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> odc_core::Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => {
            let repo = Repo::init(&cli.repo)?;
            let head = repo
                .head()?
                .map_or_else(|| "".to_string(), |hash| hash::to_hex(&hash));
            println!("initialized ODC repository at {}", cli.repo.display());
            println!("genesis {}", &head[..12.min(head.len())]);
            Ok(ExitCode::SUCCESS)
        }
        Command::Import {
            file,
            format,
            prune,
            limit,
            progress,
            html,
        } => {
            let mut repo = Repo::open(&cli.repo)?;
            if format == "mdx" {
                return import_mdx(&mut repo, &file, prune, limit, progress, &html);
            }
            let entries = if format == "csv" {
                read_csv(&file)?
            } else {
                read_jsonl(&file)?
            };
            let report = repo.import(&entries, prune)?;
            println!(
                "imported {} entries ({} written, {} removed); working set = {}",
                entries.len(),
                report.written,
                report.removed,
                report.total
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Commit {
            message,
            author,
            timestamp,
        } => {
            let mut repo = Repo::open(&cli.repo)?;
            let ts = timestamp.unwrap_or_else(now);
            let outcome = repo.commit(&message, &author, ts)?;
            println!(
                "[{}] {}",
                &hash::to_hex(&outcome.commit.hash)[..12],
                outcome.commit.message
            );
            println!(
                " {} entries, {} new blocks, {} reused blocks",
                outcome.entries, outcome.new_blocks, outcome.reused_blocks
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Log { limit, json } => {
            let repo = Repo::open(&cli.repo)?;
            let commits = repo.log(limit)?;
            if json {
                let items: Vec<_> = commits
                    .iter()
                    .map(|commit| {
                        json!({
                            "hash": hash::to_hex(&commit.hash),
                            "parent": hash::to_hex(&commit.parent),
                            "parent2": hash::to_hex(&commit.parent2),
                            "entry_count": commit.entry_count,
                            "ts": commit.ts,
                            "author": commit.author,
                            "message": commit.message,
                            "entry_root": hash::to_hex(&commit.entry_root),
                            "block_root": hash::to_hex(&commit.block_root),
                        })
                    })
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&items).unwrap_or_default()
                );
            } else {
                for commit in commits {
                    println!(
                        "[{}] {} {} <{}> ({} entries)",
                        &hash::to_hex(&commit.hash)[..12],
                        format_ts(commit.ts),
                        commit.message,
                        commit.author,
                        commit.entry_count
                    );
                }
            }
            if repo.is_shallow()? {
                eprintln!("(shallow)");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Show { word, json } => {
            let repo = Repo::open(&cli.repo)?;
            match print_resolved(&repo, &word, json)? {
                PrintOutcome::Printed => Ok(ExitCode::SUCCESS),
                PrintOutcome::NotFound => Err(odc_core::Error::InvalidArgument(format!(
                    "entry not found: {}",
                    normalize_key(&word)
                ))),
            }
        }
        Command::Lookup { word, json } => {
            let repo = Repo::open(&cli.repo)?;
            match print_resolved(&repo, &word, json)? {
                PrintOutcome::Printed => Ok(ExitCode::SUCCESS),
                PrintOutcome::NotFound => {
                    eprintln!("not found: {}", normalize_key(&word));
                    Ok(ExitCode::from(4))
                }
            }
        }
        Command::Search {
            prefix,
            limit,
            json,
        } => {
            let repo = Repo::open(&cli.repo)?;
            let hits = repo.search_prefix(&prefix, limit)?;
            if json {
                let rows: Vec<_> = hits
                    .iter()
                    .map(|(word, display)| json!({ "word": word, "display": display }))
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&rows).unwrap_or_default()
                );
            } else {
                for (word, display) in &hits {
                    if word == display {
                        println!("{word}");
                    } else {
                        println!("{word}\t{display}");
                    }
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Export { format, output } => {
            let repo = Repo::open(&cli.repo)?;
            let written = export_entries(&repo, &format, output.as_deref())?;
            eprintln!("exported {written} entries as {format}");
            Ok(ExitCode::SUCCESS)
        }
        Command::Pack {
            shallow,
            output,
            force,
        } => {
            let repo = Repo::open(&cli.repo)?;
            let out = output.unwrap_or_else(|| default_pack_path(&cli.repo));
            let report = repo.pack(&out, odc_core::pack::PackOptions { shallow, force })?;
            let kind = if report.shallow { "shallow" } else { "full" };
            println!(
                "packed {kind}: {} ({} commits, {} blocks, {} entries, {} bytes)",
                report.path.display(),
                report.commits,
                report.blocks,
                report.entries,
                report.bytes
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Diff { from, to, json } => {
            let repo = Repo::open(&cli.repo)?;
            let diff = repo.diff(&from, &to)?;
            if json {
                let stdout = std::io::stdout();
                let mut out = std::io::BufWriter::new(stdout.lock());
                out.write_all(b"{\n  ")?;
                write_json_field(&mut out, "from", &diff.from)?;
                out.write_all(b",\n  ")?;
                write_json_field(&mut out, "to", &diff.to)?;
                out.write_all(b",\n  ")?;
                write_json_string_list(&mut out, "added", &diff.added)?;
                out.write_all(b",\n  ")?;
                write_json_string_list(&mut out, "removed", &diff.removed)?;
                out.write_all(b",\n  ")?;
                write_json_string_list(&mut out, "modified", &diff.modified)?;
                out.write_all(b"\n}\n")?;
                out.flush()?;
            } else {
                println!("from {}", diff.from);
                println!("to {}", diff.to);
                println!("added: {}", diff.added.len());
                println!("removed: {}", diff.removed.len());
                println!("modified: {}", diff.modified.len());
                for word in &diff.added {
                    println!("+ {word}");
                }
                for word in &diff.removed {
                    println!("- {word}");
                }
                for word in &diff.modified {
                    println!("~ {word}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }

        Command::Gc {
            keep_last,
            dry_run,
            vacuum,
        } => {
            let mut repo = Repo::open(&cli.repo)?;
            if keep_last == 0 {
                return Err(odc_core::Error::InvalidArgument(
                    "gc requires --keep-last of at least 1".to_string(),
                ));
            }
            let report = repo.gc(keep_last, dry_run, vacuum)?;
            if report.dry_run {
                println!("gc: dry run (nothing written)");
            }
            println!("commits kept: {}", report.kept_commits);
            println!(
                "commits removed: {} ({} indexes, {} bytes)",
                report.removed_commits, report.removed_indexes, report.freed_index_bytes
            );
            println!(
                "blocks removed: {} ({} bytes)",
                report.removed_blocks, report.freed_block_bytes
            );
            println!("freed: {} bytes", report.freed_bytes());
            if report.vacuumed {
                println!("vacuumed: yes");
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Merge {
            ours,
            theirs,
            strategy,
            continue_merge,
            json,
        } => {
            let mut repo = Repo::open(&cli.repo)?;
            let strategy = Strategy::parse(&strategy)?;
            let outcome = if continue_merge {
                if ours.is_some() || theirs.is_some() {
                    return Err(odc_core::Error::InvalidArgument(
                        "--continue takes no revisions".to_string(),
                    ));
                }
                match merge::continue_merge(&mut repo, odc_core::DEFAULT_AUTHOR, now_seconds()) {
                    Ok(outcome) => outcome,
                    Err(odc_core::Error::UnresolvedConflicts(_)) => {
                        let conflicts = merge::conflicts(&repo)?;
                        print_conflicts(&conflicts, json);
                        return Ok(ExitCode::from(5));
                    }
                    Err(error) => return Err(error),
                }
            } else {
                let ours = ours.ok_or_else(|| {
                    odc_core::Error::InvalidArgument("merge needs <A> <B>".to_string())
                })?;
                let theirs = theirs.ok_or_else(|| {
                    odc_core::Error::InvalidArgument("merge needs <A> <B>".to_string())
                })?;
                merge::merge(&mut repo, &ours, &theirs, strategy)?
            };
            print_merge(&outcome, json);
            if outcome.kind == MergeKind::Conflicts {
                Ok(ExitCode::from(5))
            } else {
                Ok(ExitCode::SUCCESS)
            }
        }
        Command::Resolve {
            word,
            ours,
            theirs,
            union,
            file,
            json,
        } => {
            let mut repo = Repo::open(&cli.repo)?;
            let selected = usize::from(ours)
                + usize::from(theirs)
                + usize::from(union)
                + usize::from(file.is_some());
            if selected != 1 {
                return Err(odc_core::Error::InvalidArgument(
                    "resolve takes exactly one of --ours, --theirs, --union, --file".to_string(),
                ));
            }
            let choice = if ours {
                Choice::Ours
            } else if theirs {
                Choice::Theirs
            } else if union {
                Choice::Union
            } else {
                let path = file.expect("checked above");
                Choice::File(std::fs::read(&path)?)
            };
            let conflict = merge::resolve(&mut repo, &word, choice)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&conflict_json(&conflict)).unwrap_or_default()
                );
            } else {
                println!(
                    "resolved {} ({})",
                    conflict.word,
                    conflict
                        .resolution
                        .map(odc_core::merge::Resolution::as_str)
                        .unwrap_or("unresolved")
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Conflicts { json } => {
            let repo = Repo::open(&cli.repo)?;
            let conflicts = merge::conflicts(&repo)?;
            print_conflicts(&conflicts, json);
            Ok(ExitCode::SUCCESS)
        }
        Command::Verify => {
            let repo = Repo::open(&cli.repo)?;
            let report = repo.verify()?;
            if report.ok {
                println!(
                    "verify: OK ({} blocks, {} entries, {} commits)",
                    report.blocks, report.entries, report.commits
                );
                Ok(ExitCode::SUCCESS)
            } else {
                println!("verify: FAILED ({} problems)", report.errors.len());
                for error in &report.errors {
                    println!("  {error}");
                }
                Err(odc_core::Error::Inconsistent(format!(
                    "{} verification problems",
                    report.errors.len()
                )))
            }
        }
        Command::Status => {
            let repo = Repo::open(&cli.repo)?;
            let status = repo.status()?;
            match &status.head {
                Some(head) => println!("on commit {}", &head[..12]),
                None => println!("no commits"),
            }
            if status.is_clean() {
                println!("working set clean ({} entries)", status.total);
            } else {
                println!(
                    "changes: {} added, {} modified, {} removed",
                    status.added, status.modified, status.removed
                );
                println!("working set: {} entries", status.total);
            }
            println!(
                "index: {} entries, {} blocks, {} resources",
                status.entries,
                status.blocks,
                repo.resource_count()?
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::CheckHtml { path, json, limit } => {
            check_html(&cli.repo, path.as_deref(), json, limit)
        }
        Command::Normalize { apply, json } => normalize_html(&cli.repo, apply, json),
        Command::Patch { command } => match run_patch(&cli.repo, command) {
            Ok(code) => Ok(code),
            Err(error) => {
                // Patch failures use the same exit code as a failed lookup.
                eprintln!("error: {error}");
                Ok(ExitCode::from(4))
            }
        },
    }
}

fn run_patch(repo_dir: &std::path::Path, command: PatchCommand) -> odc_core::Result<ExitCode> {
    match command {
        PatchCommand::Create {
            base,
            target,
            output,
            json,
        } => {
            let repo = Repo::open(repo_dir)?;
            let plan = patch::create(&repo, &base, &target)?;
            let bytes = patch::encode(&plan)?;
            std::fs::write(&output, &bytes)?;
            let base_hex = hash::to_hex(&plan.base);
            let target_hex = hash::to_hex(&plan.target);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "file": output.display().to_string(),
                        "base": base_hex,
                        "target": target_hex,
                        "base_blocks": plan.base_blocks.len(),
                        "new_blocks": plan.new_blocks.len(),
                        "index_changes": plan.index_rows.len(),
                        "bytes": bytes.len(),
                    }))
                    .unwrap_or_default()
                );
            } else {
                println!(
                    "wrote {}: {} new blocks, {} base blocks, {} index changes, {} bytes",
                    output.display(),
                    plan.new_blocks.len(),
                    plan.base_blocks.len(),
                    plan.index_rows.len(),
                    bytes.len()
                );
                println!("target: {target_hex}");
            }
            Ok(ExitCode::SUCCESS)
        }
        PatchCommand::Apply { file, json } => {
            let mut repo = Repo::open(repo_dir)?;
            let bytes = std::fs::read(&file)?;
            let outcome = patch::apply(&mut repo, &bytes)?;
            match outcome {
                patch::ApplyOutcome::Applied(outcome) => {
                    let target = hash::to_hex(&outcome.target);
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "applied": true,
                                "merged": false,
                                "target": target,
                                "base_blocks": outcome.base_blocks,
                                "new_blocks": outcome.new_blocks,
                                "index_changes": outcome.index_changes,
                                "bytes": outcome.bytes,
                            }))
                            .unwrap_or_default()
                        );
                    } else {
                        println!(
                            "applied: {} new blocks, {} base blocks, {} index changes, {} bytes",
                            outcome.new_blocks,
                            outcome.base_blocks,
                            outcome.index_changes,
                            outcome.bytes
                        );
                        println!("target: {target}");
                    }
                    Ok(ExitCode::SUCCESS)
                }
                patch::ApplyOutcome::Merged {
                    patch: outcome,
                    merge: merged,
                } => {
                    let target = hash::to_hex(&outcome.target);
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&json!({
                                "applied": true,
                                "merged": true,
                                "target": target,
                                "base_blocks": outcome.base_blocks,
                                "new_blocks": outcome.new_blocks,
                                "index_changes": outcome.index_changes,
                                "bytes": outcome.bytes,
                                "merge": merge_json(merged.as_ref()),
                            }))
                            .unwrap_or_default()
                        );
                    } else {
                        println!(
                            "applied: {} new blocks, {} base blocks, {} index changes, {} bytes",
                            outcome.new_blocks,
                            outcome.base_blocks,
                            outcome.index_changes,
                            outcome.bytes
                        );
                        println!("target: {target}");
                        print_merge(merged.as_ref(), false);
                    }
                    if merged.kind == MergeKind::Conflicts {
                        return Ok(ExitCode::from(5));
                    }
                    Ok(ExitCode::SUCCESS)
                }
            }
        }
        PatchCommand::Verify { file, json } => {
            let repo = Repo::open(repo_dir)?;
            let bytes = std::fs::read(&file)?;
            let report = patch::verify(&repo, &bytes)?;
            let base = hash::to_hex(&report.base);
            let target = hash::to_hex(&report.target);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "base": base,
                        "target": target,
                        "base_blocks": report.base_blocks,
                        "new_blocks": report.new_blocks,
                        "index_changes": report.index_changes,
                        "target_present": report.target_present,
                    }))
                    .unwrap_or_default()
                );
            } else {
                println!("ok: base {base} target {target}");
                println!(
                    "{} base blocks, {} new blocks, {} index changes, target present: {}",
                    report.base_blocks,
                    report.new_blocks,
                    report.index_changes,
                    if report.target_present { "yes" } else { "no" }
                );
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
fn default_pack_path(repo: &std::path::Path) -> PathBuf {
    if repo.is_dir() {
        let name = repo.file_name().map_or_else(
            || "packed".to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let parent = repo.parent().unwrap_or_else(|| std::path::Path::new("."));
        parent.join(format!("{name}.odc"))
    } else {
        let stem = repo.file_stem().map_or_else(
            || "packed".to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let parent = repo.parent().unwrap_or_else(|| std::path::Path::new("."));
        parent.join(format!("{stem}.packed.odc"))
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn format_ts(ts: i64) -> String {
    format!("@{ts}")
}

/// What a word lookup ended up printing.
enum PrintOutcome {
    /// The entry (possibly after following link redirects) was printed.
    Printed,
    /// The dictionary carries no such word.
    NotFound,
}

/// Looks a word up through link redirects and prints the entry it reaches.
fn print_resolved(repo: &Repo, word: &str, json: bool) -> odc_core::Result<PrintOutcome> {
    match repo.resolve_link(word)? {
        LinkResolution::Found {
            requested,
            key,
            hops,
            entry,
        } => {
            if json {
                let mut value = entry_json(&entry);
                if hops > 0 {
                    value["requested_key"] = json!(requested);
                    value["redirect_hops"] = json!(hops);
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
            } else {
                if hops > 0 {
                    eprintln!("@@@LINK: {requested} -> {key} ({hops} redirects)");
                }
                print_entry(&entry);
            }
            Ok(PrintOutcome::Printed)
        }
        LinkResolution::Missing { target, .. } => Err(odc_core::Error::InvalidArgument(format!(
            "redirect target not found: {target}"
        ))),
        LinkResolution::Cycle { chain, .. } => Err(odc_core::Error::InvalidArgument(format!(
            "redirect cycle: {}",
            chain.join(" -> ")
        ))),
        LinkResolution::TooLong { chain, .. } => Err(odc_core::Error::InvalidArgument(format!(
            "redirect chain too long: {}",
            chain.join(" -> ")
        ))),
        LinkResolution::NotFound { .. } => Ok(PrintOutcome::NotFound),
    }
}

fn entry_json(entry: &Entry) -> serde_json::Value {
    let defs: Vec<_> = entry
        .defs
        .iter()
        .map(|def| {
            json!({
                "pos": def.pos,
                "sense": def.sense,
                "examples": def.examples,
                "labels": def.labels,
            })
        })
        .collect();
    json!({
        "word": entry.word,
        "display": entry.display,
        "phonetic": entry.phonetic,
        "defs": defs,
        "raw_html": entry.raw_html,
    })
}

fn print_entry(entry: &Entry) {
    println!("{}", entry.display);
    for phonetic in &entry.phonetic {
        println!("/{phonetic}/");
    }
    for (index, def) in entry.defs.iter().enumerate() {
        let pos = if def.pos.is_empty() {
            String::new()
        } else {
            format!(" ({})", def.pos)
        };
        println!("{}. {}{}", index + 1, def.sense, pos);
        for example in &def.examples {
            println!("   e.g. {example}");
        }
        if !def.labels.is_empty() {
            println!("   [{}]", def.labels.join(", "));
        }
    }
    if let Some(html) = &entry.raw_html {
        println!("raw_html: {} bytes", html.len());
    }
}

/// Turns stage-scoped core samples into one line per stage and percent.
///
/// The core already caps the callback at about ten samples per second; this
/// prints only when the stage changes or the overall percentage moves, so a
/// 180k-resource import stays readable in a terminal.
#[derive(Default)]
struct ProgressTally {
    entries_total: u64,
    entries_done: u64,
    resources_total: u64,
    resources_done: u64,
    last_percent: u64,
    last_stage: Option<odc_core::ImportStage>,
}

impl ProgressTally {
    fn observe(&mut self, sample: odc_core::ImportProgress) -> Option<String> {
        use odc_core::ImportStage;
        match sample.stage {
            ImportStage::Preparing => {
                self.resources_total = sample.total;
                self.resources_done = 0;
                self.entries_done = 0;
            }
            ImportStage::Entries => {
                self.entries_total = sample.total;
                self.entries_done = sample.done;
            }
            ImportStage::Resources => {
                self.resources_total = sample.total;
                self.resources_done = sample.done;
                self.entries_done = self.entries_total;
            }
            ImportStage::Committing => {
                self.entries_done = self.entries_total;
                self.resources_done = self.resources_total;
            }
        }
        let overall = percent(
            self.entries_done + self.resources_done,
            self.entries_total + self.resources_total,
        );
        if self.last_stage == Some(sample.stage) && overall <= self.last_percent {
            return None;
        }
        self.last_stage = Some(sample.stage);
        self.last_percent = overall;
        let line = match sample.stage {
            ImportStage::Preparing => format!(
                "progress: preparing {overall}% · resources {} · volumes {}",
                thousands(sample.total),
                sample.volume_count
            ),
            ImportStage::Entries => format!(
                "progress: entries {overall}% · {} / {}",
                thousands(sample.done),
                thousands(sample.total)
            ),
            ImportStage::Resources => {
                let volumes = if sample.volume_count > 0 {
                    format!(" · volume {}/{}", sample.volume_index, sample.volume_count)
                } else {
                    String::new()
                };
                format!(
                    "progress: resources {overall}% · {} / {}{volumes}",
                    thousands(sample.done),
                    thousands(sample.total)
                )
            }
            ImportStage::Committing => format!("progress: committing {overall}%"),
        };
        Some(line)
    }
}

fn percent(done: u64, total: u64) -> u64 {
    done.saturating_mul(100).checked_div(total).unwrap_or(100)
}

fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn import_mdx(
    repo: &mut Repo,
    path: &std::path::Path,
    prune: bool,
    limit: Option<usize>,
    progress: bool,
    html: &str,
) -> odc_core::Result<ExitCode> {
    use odc_core::mdx::MdxSource;
    use odc_core::MdxImportOptions;

    // Keep the header line ahead of the import, exactly where it used to be.
    let header = MdxSource::open(path)?.header();
    eprintln!(
        "mdx: engine {} (required {}), encoding {}, records {}{}",
        header.engine_version,
        header.required_engine_version,
        if header.encoding.is_empty() {
            "unknown"
        } else {
            header.encoding.as_str()
        },
        header.records,
        header
            .encrypted
            .as_deref()
            .map_or(String::new(), |value| format!(", encrypted={value}")),
    );

    let mut tally = ProgressTally::default();
    let html = HtmlMode::from_name(html).unwrap_or(HtmlMode::Format);
    let options = MdxImportOptions {
        skip_corrupt_blocks: true,
        prune,
        limit,
        html: HtmlOptions {
            mode: html,
            ..HtmlOptions::default()
        },
        import_mdd: true,
        mdd_volumes: None,
        progress: if progress {
            Some(Box::new(move |sample: odc_core::ImportProgress| {
                if let Some(line) = tally.observe(sample) {
                    eprintln!("{line}");
                }
            }))
        } else {
            None
        },
    };
    let report = odc_core::import_mdx(path, repo, options)?;

    if let Some(warning) = &report.scan_warning {
        eprintln!(
            "mdx: warning: {warning}; reading the file as-is \
             (corrupt-block recovery unavailable)"
        );
    }
    for block in &report.corrupt_blocks {
        let (start, end) = block.decompressed_range();
        eprintln!(
            "mdx: corrupt record block #{} at offset {} ({} B compressed, {} B declared, \
             decompressed bytes {start}..{end}): {}",
            block.ordinal,
            block.file_offset,
            block.compressed_size,
            block.declared_size,
            block.detail
        );
    }
    println!(
        "imported {} entries ({} written, {} removed); working set = {}",
        report.imported_entries, report.written, report.removed, report.working_set
    );
    if report.deduplicated + report.skipped_empty + report.skipped_oversized > 0 {
        println!(
            "mdx: {} duplicate keys suffixed, {} empty skipped, {} oversized skipped",
            report.deduplicated, report.skipped_empty, report.skipped_oversized
        );
    }
    for volume in &report.mdd_files {
        println!(
            "mdd: {} resources ({} bytes) from {}",
            volume.entries,
            volume.bytes,
            volume.path.display()
        );
    }
    for warning in &report.warnings {
        eprintln!("mdd: warning: {warning}");
    }
    if report.resource_count > 0 || !report.mdd_files.is_empty() {
        println!(
            "resources: {} ({} bytes, {} volumes)",
            report.resource_count,
            report.resource_bytes,
            report.mdd_files.len()
        );
    }
    if report.stylesheets > 0 {
        println!(
            "stylesheets: {} copied from beside the .mdx",
            report.stylesheets
        );
    }
    print_html_summary("html", html.name(), &report.html);
    if report.skipped_blocks > 0 {
        println!(
            "mdx: skipped {} corrupt record block{} ({} entries)",
            report.skipped_blocks,
            if report.skipped_blocks == 1 { "" } else { "s" },
            report.skipped_entries
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// JSON shape shared by `check-html` and `normalize`.
fn html_summary_json(summary: &HtmlImportSummary) -> serde_json::Value {
    let samples: Vec<serde_json::Value> = summary
        .samples
        .iter()
        .map(|sample| {
            json!({
                "headword": sample.headword,
                "code": sample.code,
                "message": sample.message,
            })
        })
        .collect();
    json!({
        "entries_checked": summary.entries_checked,
        "entries_changed": summary.entries_changed,
        "entries_failed": summary.entries_failed,
        "css_entries": summary.css_entries,
        "css_changed": summary.css_changed,
        "diagnostics": summary.diagnostics,
        "samples": samples,
    })
}

fn print_html_summary(prefix: &str, mode: &str, summary: &HtmlImportSummary) {
    println!(
        "{prefix}: {} checked, {} changed, {} failed (mode {mode})",
        summary.entries_checked, summary.entries_changed, summary.entries_failed
    );
    if summary.css_entries > 0 {
        println!(
            "{prefix}: {} entries with CSS, {} rewritten",
            summary.css_entries, summary.css_changed
        );
    }
    for (code, count) in &summary.diagnostics {
        println!("{prefix}: {code} {count}");
    }
    for sample in &summary.samples {
        println!(
            "{prefix} sample [{}] {}: {}",
            sample.code, sample.headword, sample.message
        );
    }
}

/// `odc check-html`: read-only HTML diagnostics for an MDX file or a repository.
fn check_html(
    repo_path: &std::path::Path,
    path: Option<&std::path::Path>,
    json: bool,
    limit: Option<usize>,
) -> odc_core::Result<ExitCode> {
    let options = HtmlOptions {
        mode: HtmlMode::Check,
        ..HtmlOptions::default()
    };
    let mut summary = HtmlImportSummary::default();
    match path {
        Some(file) => {
            let source = odc_core::mdx::MdxSource::open(file)?;
            for (seen, record) in source.records().enumerate() {
                if limit.is_some_and(|cap| seen >= cap) {
                    break;
                }
                let record = record?;
                let outcome = odc_core::html::process_html(&record.html, &options);
                summary.record(record.key.trim(), &outcome, false, options.max_samples);
            }
        }
        None => {
            let repo = Repo::open(repo_path)?;
            let mut seen = 0_usize;
            repo.for_each_entry(|entry| {
                if limit.is_some_and(|cap| seen >= cap) {
                    return Ok(());
                }
                if let Some(html) = entry.raw_html.as_deref() {
                    let outcome = odc_core::html::process_html(html, &options);
                    summary.record(&entry.display, &outcome, false, options.max_samples);
                    seen += 1;
                }
                Ok(())
            })?;
        }
    }
    if json {
        let payload = json!({
            "mode": "check",
            "summary": html_summary_json(&summary),
        });
        let line = serde_json::to_string(&payload)
            .map_err(|error| odc_core::Error::InvalidArgument(error.to_string()))?;
        println!("{line}");
    } else {
        print_html_summary("html", "check", &summary);
    }
    Ok(ExitCode::SUCCESS)
}

/// `odc normalize`: rewrite `raw_html` in the working set (dry-run by default).
fn normalize_html(
    repo_path: &std::path::Path,
    apply: bool,
    json: bool,
) -> odc_core::Result<ExitCode> {
    let options = HtmlOptions {
        mode: HtmlMode::Format,
        ..HtmlOptions::default()
    };
    let mut repo = Repo::open(repo_path)?;
    if apply {
        let status = repo.working_status()?;
        if !status.is_clean() {
            return Err(odc_core::Error::InvalidArgument(
                "normalize --apply needs a clean working set; commit or discard changes first"
                    .to_string(),
            ));
        }
    }
    let mut summary = HtmlImportSummary::default();
    let mut changed_entries: Vec<Entry> = Vec::new();
    repo.for_each_entry(|mut entry| {
        if let Some(html) = entry.raw_html.as_deref() {
            let outcome = odc_core::html::process_html(html, &options);
            let changed = outcome.changed && !outcome.failed;
            if changed {
                entry.raw_html = Some(outcome.html.clone());
                changed_entries.push(entry.clone());
            }
            summary.record(&entry.display, &outcome, changed, options.max_samples);
        }
        Ok(())
    })?;

    let mut commit = None;
    if apply && !changed_entries.is_empty() {
        repo.import(&changed_entries, false)?;
        let message = format!(
            "normalize HTML for {} entries (odc {})",
            changed_entries.len(),
            env!("CARGO_PKG_VERSION")
        );
        let outcome = repo.commit(&message, odc_core::DEFAULT_AUTHOR, now())?;
        let hex = hash::to_hex(&outcome.commit.hash);
        println!(
            "normalize: committed {} entries as [{}] {}",
            outcome.entries,
            &hex[..12],
            outcome.commit.message
        );
        commit = Some(hex);
    }

    if json {
        let payload = json!({
            "mode": "normalize",
            "applied": apply,
            "entries_updated": changed_entries.len(),
            "commit": commit,
            "summary": html_summary_json(&summary),
        });
        let line = serde_json::to_string(&payload)
            .map_err(|error| odc_core::Error::InvalidArgument(error.to_string()))?;
        println!("{line}");
    } else {
        let mode = if apply { "format" } else { "format, dry run" };
        print_html_summary("normalize", mode, &summary);
        if !apply {
            println!(
                "normalize: {} of {} entries would change (pass --apply to write)",
                changed_entries.len(),
                summary.entries_checked
            );
        } else if changed_entries.is_empty() {
            println!("normalize: no changes");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn export_entries(
    repo: &Repo,
    format: &str,
    output: Option<&std::path::Path>,
) -> odc_core::Result<usize> {
    use std::io::Write;

    let mut writer: Box<dyn Write> = match output {
        Some(path) => Box::new(std::io::BufWriter::new(std::fs::File::create(path)?)),
        None => Box::new(std::io::BufWriter::new(std::io::stdout())),
    };
    let csv = format == "csv";
    if csv {
        writeln!(writer, "word,display,pos,sense,raw_html")?;
    }
    let mut written = 0usize;
    repo.for_each_entry(|entry| {
        if csv {
            let def = entry.defs.first();
            let fields = [
                csv_field(&entry.word),
                csv_field(&entry.display),
                csv_field(def.map_or("", |item| item.pos.as_str())),
                csv_field(def.map_or("", |item| item.sense.as_str())),
                csv_field(entry.raw_html.as_deref().unwrap_or("")),
            ];
            writeln!(writer, "{}", fields.join(","))?;
        } else {
            let line = serde_json::to_string(&entry_json(&entry))
                .map_err(|error| odc_core::Error::InvalidArgument(error.to_string()))?;
            writeln!(writer, "{line}")?;
        }
        written += 1;
        Ok(())
    })?;
    writer.flush()?;
    Ok(written)
}

fn csv_field(value: &str) -> String {
    if value
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\n' | '\r'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn read_jsonl(path: &std::path::Path) -> odc_core::Result<Vec<Entry>> {
    let text = std::fs::read_to_string(path)?;
    let mut entries = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
            odc_core::Error::InvalidArgument(format!("line {}: {error}", index + 1))
        })?;
        entries.push(entry_from_json(&value)?);
    }
    Ok(entries)
}

fn entry_from_json(value: &serde_json::Value) -> odc_core::Result<Entry> {
    let object = value.as_object().ok_or_else(|| {
        odc_core::Error::InvalidArgument("entry must be a JSON object".to_string())
    })?;
    let word = object
        .get("word")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| odc_core::Error::InvalidArgument("missing word".to_string()))?;
    let mut entry = Entry::new(word);
    if let Some(display) = object.get("display").and_then(serde_json::Value::as_str) {
        entry.display = display.to_string();
    }
    if let Some(phonetic) = object.get("phonetic").and_then(serde_json::Value::as_array) {
        for item in phonetic {
            entry.phonetic.push(
                item.as_str()
                    .ok_or_else(|| {
                        odc_core::Error::InvalidArgument("phonetic must be strings".to_string())
                    })?
                    .to_string(),
            );
        }
    }
    if let Some(defs) = object.get("defs").and_then(serde_json::Value::as_array) {
        for item in defs {
            entry.defs.push(def_from_json(item)?);
        }
    }
    if let Some(raw_html) = object.get("raw_html") {
        entry.raw_html = raw_html.as_str().map(str::to_string);
    }
    if let Some(extra) = object.get("extra") {
        let extra = extra.as_object().ok_or_else(|| {
            odc_core::Error::InvalidArgument("extra must be an object".to_string())
        })?;
        for (key, item) in extra {
            entry.extra.push((key.clone(), json_to_cbor(item)?));
        }
    }
    for (key, item) in object {
        if matches!(
            key.as_str(),
            "word" | "display" | "phonetic" | "defs" | "raw_html" | "extra"
        ) {
            continue;
        }
        entry.extra.push((key.clone(), json_to_cbor(item)?));
    }
    entry.extra.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entry)
}

fn def_from_json(value: &serde_json::Value) -> odc_core::Result<Def> {
    let object = value
        .as_object()
        .ok_or_else(|| odc_core::Error::InvalidArgument("def must be an object".to_string()))?;
    let mut def = Def::default();
    if let Some(pos) = object.get("pos").and_then(serde_json::Value::as_str) {
        def.pos = pos.to_string();
    }
    if let Some(sense) = object.get("sense").and_then(serde_json::Value::as_str) {
        def.sense = sense.to_string();
    }
    for (field, target) in [("examples", &mut def.examples), ("labels", &mut def.labels)] {
        if let Some(items) = object.get(field).and_then(serde_json::Value::as_array) {
            for item in items {
                target.push(
                    item.as_str()
                        .ok_or_else(|| {
                            odc_core::Error::InvalidArgument(format!("{field} must be strings"))
                        })?
                        .to_string(),
                );
            }
        }
    }
    Ok(def)
}

fn json_to_cbor(value: &serde_json::Value) -> odc_core::Result<cbor::Value> {
    match value {
        serde_json::Value::Null => Ok(cbor::Value::Null),
        serde_json::Value::Bool(flag) => Ok(cbor::Value::Bool(*flag)),
        serde_json::Value::Number(number) => {
            number.as_i64().map(cbor::Value::Int).ok_or_else(|| {
                odc_core::Error::InvalidArgument(format!(
                    "only integers are allowed in extra fields, got {number}"
                ))
            })
        }
        serde_json::Value::String(text) => Ok(cbor::Value::Text(text.clone())),
        serde_json::Value::Array(items) => Ok(cbor::Value::Array(
            items
                .iter()
                .map(json_to_cbor)
                .collect::<odc_core::Result<_>>()?,
        )),
        serde_json::Value::Object(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (key, item) in map {
                pairs.push((cbor::Value::Text(key.clone()), json_to_cbor(item)?));
            }
            Ok(cbor::Value::Map(pairs))
        }
    }
}

fn read_csv(path: &std::path::Path) -> odc_core::Result<Vec<Entry>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| odc_core::Error::InvalidArgument("empty CSV file".to_string()))?;
    let columns: Vec<String> = split_csv_line(header)?
        .into_iter()
        .map(|column| column.trim().to_lowercase())
        .collect();
    let index = |name: &str| columns.iter().position(|column| column == name);
    let word_at = index("word")
        .ok_or_else(|| odc_core::Error::InvalidArgument("CSV header needs word".to_string()))?;
    let display_at = index("display");
    let definition_at = index("definition").or_else(|| index("sense"));
    let pos_at = index("pos");
    let raw_html_at = index("raw_html");
    let mut entries = Vec::new();
    for line in lines {
        let fields = split_csv_line(line)?;
        let word = fields.get(word_at).cloned().unwrap_or_default();
        if word.is_empty() {
            continue;
        }
        let mut entry = Entry::new(word);
        if let Some(position) = display_at {
            if let Some(display) = fields.get(position) {
                if !display.is_empty() {
                    entry.display = display.clone();
                }
            }
        }
        let sense = definition_at
            .and_then(|position| fields.get(position).cloned())
            .unwrap_or_default();
        let pos = pos_at
            .and_then(|position| fields.get(position).cloned())
            .unwrap_or_default();
        let def = Def {
            pos,
            sense,
            ..Def::default()
        };
        entry.defs.push(def);
        if let Some(html) = raw_html_at
            .and_then(|position| fields.get(position))
            .filter(|value| !value.is_empty())
        {
            entry.raw_html = Some(html.clone());
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn split_csv_line(line: &str) -> odc_core::Result<Vec<String>> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '"' if quoted && characters.peek() == Some(&'"') => {
                current.push('"');
                let _ = characters.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(std::mem::take(&mut current));
            }
            other => current.push(other),
        }
    }
    if quoted {
        return Err(odc_core::Error::InvalidArgument(
            "unterminated CSV quote".to_string(),
        ));
    }
    fields.push(current);
    Ok(fields)
}

/// Write `"key": value` with the streaming JSON encoder.
fn write_json_field<W: std::io::Write, T: serde::Serialize + ?Sized>(
    out: &mut W,
    key: &str,
    value: &T,
) -> odc_core::Result<()> {
    write!(out, "\"{key}\": ")?;
    serde_json::to_writer(&mut *out, value).map_err(|error| odc_core::Error::Io(error.to_string()))
}

/// Write a JSON array of strings, one element at a time.
fn write_json_string_list<W: std::io::Write>(
    out: &mut W,
    key: &str,
    words: &[String],
) -> odc_core::Result<()> {
    write!(out, "\"{key}\": [")?;
    for (index, word) in words.iter().enumerate() {
        if index > 0 {
            out.write_all(b", ")?;
        }
        serde_json::to_writer(&mut *out, word)
            .map_err(|error| odc_core::Error::Io(error.to_string()))?;
    }
    out.write_all(b"]")?;
    Ok(())
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn hex_option(digest: Option<[u8; 32]>) -> serde_json::Value {
    digest.map_or(serde_json::Value::Null, |hash| json!(hash::to_hex(&hash)))
}

fn conflict_json(conflict: &merge::Conflict) -> serde_json::Value {
    json!({
        "word": conflict.word,
        "fields": conflict.fields,
        "base": hex_option(conflict.base),
        "ours": hex_option(conflict.ours),
        "theirs": hex_option(conflict.theirs),
        "resolution": conflict.resolution.map(merge::Resolution::as_str),
    })
}

fn merge_json(outcome: &MergeOutcome) -> serde_json::Value {
    let kind = match outcome.kind {
        MergeKind::UpToDate => "up-to-date",
        MergeKind::FastForward => "fast-forward",
        MergeKind::Merged => "merged",
        MergeKind::Conflicts => "conflicts",
    };
    let conflicts: Vec<_> = outcome.conflicts.iter().map(conflict_json).collect();
    json!({
        "kind": kind,
        "strategy": outcome.strategy.as_str(),
        "base": hex_option(outcome.base),
        "ours": hash::to_hex(&outcome.ours),
        "theirs": hash::to_hex(&outcome.theirs),
        "commit": hex_option(outcome.commit.as_ref().map(|commit| commit.hash)),
        "auto_merged": outcome.auto_merged,
        "new_blocks": outcome.new_blocks,
        "reused_blocks": outcome.reused_blocks,
        "conflicts": conflicts,
    })
}

fn short_hash(digest: &[u8; 32]) -> String {
    hash::to_hex(digest)[..12].to_string()
}

fn short_text(text: &str) -> String {
    text.chars().take(12).collect()
}

fn print_merge(outcome: &MergeOutcome, json: bool) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&merge_json(outcome)).unwrap_or_default()
        );
        return;
    }
    let base = outcome.base.map_or_else(
        || "none".to_string(),
        |hash| short_text(&hash::to_hex(&hash)),
    );
    match outcome.kind {
        MergeKind::UpToDate => {
            println!("already up to date with {}", short_hash(&outcome.theirs));
        }
        MergeKind::FastForward => {
            println!(
                "fast-forward to {} (was {})",
                short_hash(&outcome.theirs),
                short_hash(&outcome.ours)
            );
        }
        MergeKind::Merged => {
            let commit = outcome
                .commit
                .as_ref()
                .map_or(outcome.ours, |commit| commit.hash);
            println!(
                "merge commit {} (parent2 {}, base {})",
                short_hash(&commit),
                short_hash(&outcome.theirs),
                base
            );
        }
        MergeKind::Conflicts => println!(
            "conflicts: {} ({} auto-merged entries); resolve them or run merge --continue",
            outcome.conflicts.len(),
            outcome.auto_merged
        ),
    }
    for conflict in &outcome.conflicts {
        print_conflict_line(conflict);
    }
}

fn print_conflict_line(conflict: &merge::Conflict) {
    if conflict.fields.is_empty() {
        println!("  {}", conflict.word);
    } else {
        println!("  {}: {}", conflict.word, conflict.fields.join(", "));
    }
}

fn print_conflicts(conflicts: &[merge::Conflict], json: bool) {
    if json {
        let items: Vec<_> = conflicts.iter().map(conflict_json).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&items).unwrap_or_default()
        );
        return;
    }
    if conflicts.is_empty() {
        println!("no conflicts");
        return;
    }
    for conflict in conflicts {
        print_conflict_line(conflict);
        if let Some(resolution) = conflict.resolution {
            println!("    resolved with {}", resolution.as_str());
        }
    }
}
