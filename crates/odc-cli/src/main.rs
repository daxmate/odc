//! `odc` command line interface (M0).

use clap::{Parser, Subcommand};
use odc_core::cbor;
use odc_core::entry::{normalize_key, Def, Entry};
use odc_core::hash;
use odc_core::html::{HtmlImportSummary, HtmlMode, HtmlOptions};
use odc_core::link::LinkResolution;
use odc_core::mdx::MdxSource;
use odc_core::merge::{self, Choice, MergeKind, MergeOutcome, Strategy};
use odc_core::patch;
use odc_core::release::{self, PublishOptions};
use odc_core::repo::{Repo, TRUST_SOURCE_MANUAL};
use odc_core::signing;
use odc_core::survey::{SurveyOptions, Surveyor};
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
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
        /// Largest record body accepted, in bytes (MDX only).
        ///
        /// Defaults to 4194304 (4 MiB); values above 16777216 (16 MiB) are
        /// refused.
        #[arg(long, value_name = "BYTES")]
        max_record_bytes: Option<usize>,
        /// Print progress to stderr (MDX only).
        #[arg(long)]
        progress: bool,
        /// HTML handling for imported entry bodies (MDX only).
        #[arg(long, value_parser = ["format", "check", "off"], default_value = "format")]
        html: String,
        /// Profile applied to every record (MDX only).
        ///
        /// A record the profile does not claim keeps the built-in HTML pass
        /// and is counted as unadapted in the report.
        #[arg(long, value_name = "PATH")]
        profile: Option<PathBuf>,
        /// Emit the import report as JSON (MDX only).
        #[arg(long)]
        json: bool,
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
    /// Deepen a shallow repository from another ODC repository (offline).
    Fetch {
        /// Source repository: a repository directory or a packed .odc file.
        #[arg(long)]
        from: PathBuf,
        /// Total commits visible from HEAD once the fetch finishes.
        #[arg(long)]
        depth: u32,
        /// Print the result as JSON.
        #[arg(long)]
        json: bool,
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
    /// Generate an Ed25519 signing keypair.
    Keygen {
        /// Output directory (default $ODC_KEY_DIR or ~/.config/odc/keys).
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Key name (default `default`).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Overwrite existing key files.
        #[arg(long)]
        force: bool,
    },
    /// Sign a file, writing a detached `<FILE>.sig` sidecar.
    Sign {
        /// File to sign.
        file: PathBuf,
        /// Private key (default `<key dir>/default.key`).
        #[arg(long, value_name = "PATH")]
        key: Option<PathBuf>,
        /// Signer name recorded in the sidecar.
        #[arg(long, value_name = "NAME")]
        signer: Option<String>,
        /// Signature output path (default `<FILE>.sig`).
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Overwrite an existing signature file.
        #[arg(long)]
        force: bool,
    },
    /// Verify a detached signature against a file.
    VerifySignature {
        /// File to check.
        file: PathBuf,
        /// Signature file (default `<FILE>.sig`).
        #[arg(long, value_name = "PATH")]
        sig: Option<PathBuf>,
        /// Public key file written by `odc keygen`.
        #[arg(long = "pub", value_name = "PATH")]
        pub_file: Option<PathBuf>,
        /// Public key as 64 hex characters.
        #[arg(long, value_name = "HEX")]
        pubkey: Option<String>,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Publish a signed release bundle.
    Publish {
        /// Output directory for the release files.
        #[arg(short, long, value_name = "DIR")]
        out: PathBuf,
        /// Dictionary name (default: the repository directory name).
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Release label used in file names, e.g. 2026.11.
        #[arg(long, value_name = "LABEL")]
        label: String,
        /// Also publish an incremental patch from this revision.
        #[arg(long, value_name = "COMMIT")]
        base: Option<String>,
        /// Private key (default `<key dir>/default.key`).
        #[arg(long, value_name = "PATH")]
        key: Option<PathBuf>,
        /// Signer name recorded in the sidecars.
        #[arg(long, value_name = "NAME")]
        signer: Option<String>,
        /// Free-form note stored in the manifest.
        #[arg(long, value_name = "TEXT")]
        notes: Option<String>,
        /// Overwrite existing release files.
        #[arg(long)]
        force: bool,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Verify a release directory.
    VerifyRelease {
        /// Release directory containing release.json.
        dir: PathBuf,
        /// Public key file to pin the publisher.
        #[arg(long = "pub", value_name = "PATH")]
        pub_file: Option<PathBuf>,
        /// Public key as 64 hex characters to pin the publisher.
        #[arg(long, value_name = "HEX")]
        pubkey: Option<String>,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Manage the publisher trust store.
    Trust {
        #[command(subcommand)]
        command: TrustCommand,
    },
    /// Patch creation and application (specified, implemented in M3).
    Patch {
        #[command(subcommand)]
        command: PatchCommand,
    },
    /// Measure a dictionary's HTML dialect without writing any rule.
    Survey {
        /// Input MDX file.
        file: PathBuf,
        /// Directory for survey.md, survey.json and samples/.
        #[arg(long, value_name = "DIR", default_value = "odc-survey")]
        out: PathBuf,
        /// Maximum records to read (0 = every record).
        #[arg(long, default_value_t = 2000)]
        limit: usize,
        /// CSS subset selector for the entry container.
        #[arg(long, value_name = "SEL")]
        entry_selector: Option<String>,
        /// Share of entries a slot kind needs to shape a template.
        #[arg(long, value_name = "RATIO", default_value_t = 0.01)]
        core_threshold: f64,
        /// Write a stratified sample of body records as JSONL for
        /// `profile_compare --source`.
        #[arg(long, value_name = "PATH")]
        dump_jsonl: Option<PathBuf>,
        /// Records to keep in the sample.
        ///
        /// `0` keeps every body record (full dump).
        #[arg(long, value_name = "N", default_value_t = 500)]
        dump_limit: usize,
        /// Fixed stride inside every bucket (default: derived from size / quota).
        #[arg(long, value_name = "N")]
        dump_stride: Option<usize>,
        /// Print the report as JSON instead of a summary.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
enum TrustCommand {
    /// Trust a publisher key.
    Add {
        /// Public key file written by keygen.
        #[arg(long = "pub", value_name = "PATH")]
        pub_file: Option<PathBuf>,
        /// Public key as 64 hex characters.
        #[arg(long, value_name = "HEX")]
        pubkey: Option<String>,
        /// Display name for the publisher.
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// List trusted publishers.
    List {
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
    },
    /// Forget a publisher key.
    Remove {
        /// Public key as 64 hex characters.
        public_key: String,
        /// Print a JSON report.
        #[arg(long)]
        json: bool,
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
        /// Verify this sidecar signature before applying anything.
        #[arg(long, value_name = "PATH")]
        sig: Option<PathBuf>,
        /// Trust an unknown publisher on first use (empty trust store only).
        #[arg(long)]
        trust_new: bool,
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
            max_record_bytes,
            progress,
            html,
            profile,
            json,
        } => {
            let mut repo = Repo::open(&cli.repo)?;
            if format != "mdx" {
                if profile.is_some() {
                    return Err(odc_core::Error::InvalidArgument(
                        "--profile is only supported with --format mdx".to_string(),
                    ));
                }
                if json {
                    return Err(odc_core::Error::InvalidArgument(
                        "--json is only supported with --format mdx".to_string(),
                    ));
                }
            }
            if format == "mdx" {
                return import_mdx(MdxImportArgs {
                    repo: &mut repo,
                    path: &file,
                    prune,
                    limit,
                    max_record_bytes,
                    progress,
                    html: &html,
                    profile_path: profile.as_deref(),
                    json,
                });
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
        Command::Fetch { from, depth, json } => {
            let mut repo = Repo::open(&cli.repo)?;
            let report = odc_core::fetch::deepen(&mut repo, &from, depth)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "depth": report.depth_requested,
                        "commits_added": report.commits_added,
                        "commit_index_added": report.commit_index_added,
                        "blocks_added": report.blocks_added,
                        "complete": report.complete,
                    }))
                    .unwrap_or_default()
                );
            } else if report.complete {
                println!(
                    "fetched to depth {}: {} commits and {} blocks added, history complete",
                    report.depth_requested, report.commits_added, report.blocks_added
                );
            } else {
                println!(
                    "fetched to depth {}: {} commits and {} blocks added (still shallow)",
                    report.depth_requested, report.commits_added, report.blocks_added
                );
            }
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
        Command::Keygen { out, name, force } => run_keygen(out, name, force),
        Command::Sign {
            file,
            key,
            signer,
            out,
            force,
        } => run_sign(
            &file,
            key.as_deref(),
            signer.as_deref(),
            out.as_deref(),
            force,
        ),
        Command::VerifySignature {
            file,
            sig,
            pub_file,
            pubkey,
            json,
        } => run_verify_signature(
            &file,
            sig.as_deref(),
            pub_file.as_deref(),
            pubkey.as_deref(),
            json,
        ),
        Command::Publish {
            out,
            name,
            label,
            base,
            key,
            signer,
            notes,
            force,
            json,
        } => run_publish(
            &cli.repo,
            &out,
            PublishArgs {
                name,
                label,
                base,
                key,
                signer,
                notes,
                force,
                json,
            },
        ),
        Command::Survey {
            file,
            out,
            limit,
            entry_selector,
            core_threshold,
            dump_jsonl,
            dump_limit,
            dump_stride,
            json,
        } => run_survey(
            &file,
            &out,
            limit,
            entry_selector,
            core_threshold,
            dump_jsonl.as_deref(),
            dump_limit,
            dump_stride,
            json,
        ),
        Command::VerifyRelease {
            dir,
            pub_file,
            pubkey,
            json,
        } => run_verify_release(
            &cli.repo,
            &dir,
            pub_file.as_deref(),
            pubkey.as_deref(),
            json,
        ),
        Command::Trust { command } => run_trust(&cli.repo, command),
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
        PatchCommand::Apply {
            file,
            sig,
            trust_new,
            json,
        } => {
            let mut repo = Repo::open(repo_dir)?;
            let bytes = std::fs::read(&file)?;
            if let Some(signature_path) = &sig {
                release::verify_patch_signature(&mut repo, &file, signature_path, trust_new)?;
            }
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

fn import_mdx(args: MdxImportArgs<'_>) -> odc_core::Result<ExitCode> {
    use odc_core::mdx::MdxSource;
    use odc_core::MdxImportOptions;

    let MdxImportArgs {
        repo,
        path,
        prune,
        limit,
        max_record_bytes,
        progress,
        html,
        profile_path,
        json,
    } = args;

    let profile = match profile_path {
        Some(path) => Some(load_profile(path)?),
        None => None,
    };

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
    let max_record_bytes = max_record_bytes.unwrap_or(odc_core::DEFAULT_MAX_RECORD_BYTES);
    let options = MdxImportOptions {
        skip_corrupt_blocks: true,
        prune,
        limit,
        max_record_bytes,
        html: HtmlOptions {
            mode: html,
            ..HtmlOptions::default()
        },
        import_mdd: true,
        mdd_volumes: None,
        profile,
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

    if json {
        let payload = import_report_json(&report);
        let text = serde_json::to_string_pretty(&payload).map_err(|error| {
            odc_core::Error::InvalidArgument(format!("cannot encode the import report: {error}"))
        })?;
        println!("{text}");
        return Ok(ExitCode::SUCCESS);
    }

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
    for record in &report.skipped_oversized_samples {
        println!(
            "mdx: skipped oversized record {:?} ({} bytes, limit {} bytes)",
            record.key, record.bytes, max_record_bytes
        );
    }
    if report.skipped_oversized > report.skipped_oversized_samples.len() {
        println!(
            "mdx: ... and {} more oversized records not listed",
            report.skipped_oversized - report.skipped_oversized_samples.len()
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
    if let Some(contract) = &report.contract {
        println!(
            "imported {} records | coverage {}/{} ({:.1}%) | mapped four-dim {}/{} | unadapted {}",
            report.imported_entries,
            contract.adapted,
            contract.total,
            contract.coverage() * 100.0,
            contract.adapted_all_pass,
            contract.adapted,
            contract.unadapted
        );
    }
    if report.p0_failed > 0 {
        println!(
            "mdx: {} record(s) missed a P0 rule: {}",
            report.p0_failed,
            report.p0_failed_samples.join(", ")
        );
    }
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

/// Everything `odc import --format mdx` needs, gathered from the command line.
struct MdxImportArgs<'a> {
    repo: &'a mut Repo,
    path: &'a std::path::Path,
    prune: bool,
    limit: Option<usize>,
    max_record_bytes: Option<usize>,
    progress: bool,
    html: &'a str,
    profile_path: Option<&'a std::path::Path>,
    json: bool,
}

/// `odc import --json`: the report as one object.
fn import_report_json(report: &odc_core::MdxImportReport) -> serde_json::Value {
    json!({
        "imported_entries": report.imported_entries,
        "written": report.written,
        "removed": report.removed,
        "working_set": report.working_set,
        "deduplicated": report.deduplicated,
        "skipped_empty": report.skipped_empty,
        "skipped_oversized": report.skipped_oversized,
        "skipped_blocks": report.skipped_blocks,
        "skipped_entries": report.skipped_entries,
        "corrupt_blocks": report.corrupt_blocks.len(),
        "scan_warning": report.scan_warning.as_deref(),
        "resources": {
            "count": report.resource_count,
            "bytes": report.resource_bytes,
            "volumes": report.mdd_files.len(),
        },
        "stylesheets": report.stylesheets,
        "html": html_summary_json(&report.html),
        "contract": report
            .contract
            .as_ref()
            .map(odc_core::profile::contract::ContractSummary::to_json),
        "p0_failed": report.p0_failed,
        "p0_failed_samples": &report.p0_failed_samples,
    })
}

/// Read and parse the profile named by `--profile`.
fn load_profile(path: &std::path::Path) -> odc_core::Result<odc_core::profile::Profile> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        odc_core::Error::InvalidArgument(format!("cannot read profile {}: {error}", path.display()))
    })?;
    odc_core::profile::Profile::from_json(&text).map_err(|error| {
        odc_core::Error::InvalidArgument(format!("invalid profile {}: {error}", path.display()))
    })
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

fn key_dir() -> odc_core::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("ODC_KEY_DIR") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let home = std::env::var_os("HOME").ok_or_else(|| {
        odc_core::Error::InvalidArgument("neither ODC_KEY_DIR nor HOME is set".to_string())
    })?;
    Ok(PathBuf::from(home).join(".config").join("odc").join("keys"))
}

struct PublishArgs {
    name: Option<String>,
    label: String,
    base: Option<String>,
    key: Option<PathBuf>,
    signer: Option<String>,
    notes: Option<String>,
    force: bool,
    json: bool,
}

fn default_release_name(repo_dir: &std::path::Path) -> String {
    if repo_dir.is_dir() {
        return repo_dir
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .map_or_else(|| "dictionary".to_string(), str::to_string);
    }
    repo_dir
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .map_or_else(|| "dictionary".to_string(), str::to_string)
}

fn pinned_key(
    pub_file: Option<&std::path::Path>,
    pubkey: Option<&str>,
) -> std::result::Result<Option<[u8; 32]>, String> {
    match (pub_file, pubkey) {
        (Some(_), Some(_)) => Err("--pub and --pubkey are mutually exclusive".to_string()),
        (None, None) => Ok(None),
        (Some(path), None) => {
            let text = std::fs::read_to_string(path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let key = signing::parse_public_key_file(&text).map_err(|error| error.to_string())?;
            Ok(Some(key))
        }
        (None, Some(hex)) => {
            let key = signing::hex_bytes::<32>(hex.trim(), "public_key")
                .map_err(|error| error.to_string())?;
            Ok(Some(key))
        }
    }
}

fn run_publish(
    repo_dir: &std::path::Path,
    out: &std::path::Path,
    args: PublishArgs,
) -> odc_core::Result<ExitCode> {
    let repo = Repo::open(repo_dir)?;
    let name = args.name.unwrap_or_else(|| default_release_name(repo_dir));
    let key_path = match args.key {
        Some(path) => path,
        None => key_dir()?.join("default.key"),
    };
    let key_pem = std::fs::read_to_string(&key_path)?;
    let options = PublishOptions {
        name,
        label: args.label,
        base: args.base,
        key_pem,
        signer: args.signer,
        notes: args.notes,
        force: args.force,
    };
    let report = release::publish(&repo, out, &options)?;
    if args.json {
        let artifacts: Vec<serde_json::Value> = report
            .artifacts
            .iter()
            .map(|artifact| {
                json!({
                    "kind": artifact.kind,
                    "path": artifact.path,
                    "size": artifact.size,
                    "blake3": artifact.blake3,
                })
            })
            .collect();
        let payload = json!({
            "out_dir": report.out_dir.display().to_string(),
            "manifest": report.manifest_path.display().to_string(),
            "head": report.head,
            "base": report.base,
            "artifacts": artifacts,
            "publisher": { "public_key": report.public_key },
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "published {} -> {} (manifest {})",
            report.head,
            report.out_dir.display(),
            report.manifest_path.display()
        );
        for artifact in &report.artifacts {
            println!(
                "  {} {} ({} bytes, blake3 {})",
                artifact.kind, artifact.path, artifact.size, artifact.blake3
            );
        }
        println!("publisher: {}", report.public_key);
    }
    Ok(ExitCode::SUCCESS)
}

fn run_keygen(
    out: Option<PathBuf>,
    name: Option<String>,
    force: bool,
) -> odc_core::Result<ExitCode> {
    let dir = match out {
        Some(dir) => dir,
        None => key_dir()?,
    };
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
        set_file_mode(&dir, 0o700)?;
    }
    let name = name.unwrap_or_else(|| "default".to_string());
    if name.is_empty()
        || name
            .chars()
            .any(|ch| ch == '/' || ch == '\\' || ch.is_whitespace())
    {
        return Err(odc_core::Error::InvalidArgument(format!(
            "invalid key name {name:?}"
        )));
    }
    let key_path = dir.join(format!("{name}.key"));
    let public_path = dir.join(format!("{name}.pub"));
    if !force && (key_path.exists() || public_path.exists()) {
        return Err(odc_core::Error::InvalidArgument(format!(
            "{} already exists; use --force to overwrite",
            key_path.display()
        )));
    }
    let keypair = signing::generate_keypair()?;
    write_private_key(&key_path, &keypair.private_key_pem)?;
    let public_text = signing::public_key_file_text(&keypair.public_key_hex, Some(&name))?;
    std::fs::write(&public_path, public_text)?;
    set_file_mode(&public_path, 0o644)?;
    println!("private key: {}", key_path.display());
    println!("public key: {}", public_path.display());
    println!("public key hex: {}", keypair.public_key_hex);
    Ok(ExitCode::SUCCESS)
}

fn run_sign(
    file: &std::path::Path,
    key: Option<&std::path::Path>,
    signer: Option<&str>,
    out: Option<&std::path::Path>,
    force: bool,
) -> odc_core::Result<ExitCode> {
    let key_path = match key {
        Some(path) => path.to_path_buf(),
        None => key_dir()?.join("default.key"),
    };
    let out_path = out.map_or_else(
        || signing::signature_path(file),
        std::path::Path::to_path_buf,
    );
    if !force && out_path.exists() {
        return Err(odc_core::Error::InvalidArgument(format!(
            "{} already exists; use --force to overwrite",
            out_path.display()
        )));
    }
    let key_pem = std::fs::read_to_string(&key_path)?;
    let signature = signing::sign_file(file, &key_pem, signer)?;
    signing::write_signature_file(&out_path, &signature)?;
    let mut report = json!({
        "version": signature.version,
        "algorithm": signature.algorithm,
        "public_key": signature.public_key,
        "signature": signature.signature,
        "target": {
            "hash_algorithm": signature.target.hash_algorithm,
            "hash": signature.target.hash,
            "size": signature.target.size,
        },
    });
    if let Some(signer) = &signature.signer {
        report["signer"] = json!(signer);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );
    Ok(ExitCode::SUCCESS)
}

fn run_verify_signature(
    file: &std::path::Path,
    sig: Option<&std::path::Path>,
    pub_file: Option<&std::path::Path>,
    pubkey: Option<&str>,
    json: bool,
) -> odc_core::Result<ExitCode> {
    if pub_file.is_some() && pubkey.is_some() {
        eprintln!("error: --pub and --pubkey are mutually exclusive");
        return Ok(ExitCode::from(2));
    }
    let expected = match (pub_file, pubkey) {
        (Some(path), None) => match std::fs::read_to_string(path) {
            Ok(text) => match signing::parse_public_key_file(&text) {
                Ok(key) => Some(key),
                Err(error) => {
                    eprintln!("error: {}: {error}", path.display());
                    return Ok(ExitCode::from(2));
                }
            },
            Err(error) => {
                eprintln!("error: cannot read {}: {error}", path.display());
                return Ok(ExitCode::from(2));
            }
        },
        (None, Some(hex)) => match signing::hex_bytes::<32>(hex, "public_key") {
            Ok(key) => Some(key),
            Err(error) => {
                eprintln!("error: {error}");
                return Ok(ExitCode::from(2));
            }
        },
        (None, None) => None,
        (Some(_), Some(_)) => unreachable!("checked above"),
    };

    let data = match std::fs::read(file) {
        Ok(data) => data,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", file.display());
            return Ok(ExitCode::from(2));
        }
    };
    let sig_path = sig.map_or_else(
        || signing::signature_path(file),
        std::path::Path::to_path_buf,
    );
    let sig_text = match std::fs::read_to_string(&sig_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("error: cannot read {}: {error}", sig_path.display());
            return Ok(ExitCode::from(2));
        }
    };
    let signature = match signing::SignatureV1::from_json(&sig_text) {
        Ok(signature) => signature,
        Err(error) => return Ok(report_invalid(&error, json)),
    };
    match signing::verify_bytes_with_key(&data, &signature, expected.as_ref()) {
        Ok(verified) => {
            if json {
                let mut report = json!({
                    "valid": true,
                    "public_key": hash::to_hex(&verified.public_key),
                    "created_at": verified.created_at,
                    "hash": hash::to_hex(&verified.hash),
                    "size": verified.size,
                });
                if let Some(signer) = &verified.signer {
                    report["signer"] = json!(signer);
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
            } else {
                let signer = verified
                    .signer
                    .as_deref()
                    .map_or_else(String::new, |name| format!(" by {name}"));
                println!(
                    "verify-signature: OK ({}): {} bytes, hash {}, key {}{}, signed {}",
                    file.display(),
                    verified.size,
                    hash::to_hex(&verified.hash),
                    &hash::to_hex(&verified.public_key)[..16],
                    signer,
                    verified.created_at
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => Ok(report_invalid(&error, json)),
    }
}

fn publisher_json(publisher: &odc_core::repo::Publisher) -> serde_json::Value {
    json!({
        "public_key": publisher.public_key,
        "name": publisher.name,
        "added_at": publisher.added_at,
        "source": publisher.source,
    })
}

fn run_verify_release(
    repo_dir: &std::path::Path,
    dir: &std::path::Path,
    pub_file: Option<&std::path::Path>,
    pubkey: Option<&str>,
    json: bool,
) -> odc_core::Result<ExitCode> {
    let pinned = match pinned_key(pub_file, pubkey) {
        Ok(value) => value,
        Err(message) => {
            eprintln!("error: {message}");
            return Ok(ExitCode::from(2));
        }
    };
    let report = match release::verify_release(dir, pinned.as_ref()) {
        Ok(report) => report,
        Err(error) => return Ok(report_invalid(&error, json)),
    };
    let trust = if pinned.is_some() {
        "pinned"
    } else {
        let repo = Repo::open(repo_dir)?;
        if let Err(error) = release::require_trusted_publisher(&repo, &report.publisher_key) {
            return Ok(report_invalid(&error, json));
        }
        "trusted"
    };
    if json {
        let artifacts: Vec<serde_json::Value> = report
            .artifacts
            .iter()
            .map(|artifact| {
                json!({
                    "kind": artifact.kind,
                    "path": artifact.path,
                    "size": artifact.size,
                    "blake3": artifact.blake3,
                })
            })
            .collect();
        let payload = json!({
            "valid": true,
            "dir": report.dir.display().to_string(),
            "name": report.name,
            "label": report.label,
            "head": report.head,
            "base": report.base,
            "created_at": report.created_at,
            "publisher": {
                "public_key": report.publisher_key,
                "name": report.publisher_name,
            },
            "trust": trust,
            "artifacts": artifacts,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "verify-release: OK ({} {}): {} artifacts, publisher {}, trust {trust}",
            report.name,
            report.label,
            report.artifacts.len(),
            &report.publisher_key[..16]
        );
        println!("head: {}", report.head);
        for artifact in &report.artifacts {
            println!(
                "  {} {} ({} bytes, blake3 {})",
                artifact.kind, artifact.path, artifact.size, artifact.blake3
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_trust(repo_dir: &std::path::Path, command: TrustCommand) -> odc_core::Result<ExitCode> {
    match command {
        TrustCommand::Add {
            pub_file,
            pubkey,
            name,
            json,
        } => {
            let key = match pinned_key(pub_file.as_deref(), pubkey.as_deref()) {
                Ok(Some(key)) => key,
                Ok(None) => {
                    eprintln!("error: --pub or --pubkey is required");
                    return Ok(ExitCode::from(2));
                }
                Err(message) => {
                    eprintln!("error: {message}");
                    return Ok(ExitCode::from(2));
                }
            };
            let mut repo = Repo::open(repo_dir)?;
            let hex = signing::hex_encode(&key);
            let name = name.unwrap_or_default();
            match repo.trust_publisher(&hex, &name, TRUST_SOURCE_MANUAL)? {
                odc_core::repo::TrustOutcome::Added(publisher) => {
                    if json {
                        let payload = json!({
                            "outcome": "added",
                            "publisher": publisher_json(&publisher),
                        });
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&payload).unwrap_or_default()
                        );
                    } else {
                        println!("trusted {} (source manual)", publisher.public_key);
                    }
                }
                odc_core::repo::TrustOutcome::AlreadyTrusted(publisher) => {
                    if json {
                        let payload = json!({
                            "outcome": "already_trusted",
                            "publisher": publisher_json(&publisher),
                        });
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&payload).unwrap_or_default()
                        );
                    } else {
                        println!("already trusted: {}", publisher.public_key);
                    }
                }
                odc_core::repo::TrustOutcome::NameConflict { existing } => {
                    let error = odc_core::Error::PublisherUntrusted(format!(
                        "name {:?} is already trusted for {}",
                        existing.name, existing.public_key
                    ));
                    return Ok(report_invalid(&error, json));
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        TrustCommand::List { json } => {
            let repo = Repo::open(repo_dir)?;
            let publishers = repo.publishers()?;
            if json {
                let items: Vec<serde_json::Value> = publishers.iter().map(publisher_json).collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "publishers": items }))
                        .unwrap_or_default()
                );
            } else if publishers.is_empty() {
                println!("no publishers trusted");
            } else {
                for publisher in &publishers {
                    println!(
                        "{} {} ({}, added {})",
                        publisher.public_key, publisher.name, publisher.source, publisher.added_at
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        TrustCommand::Remove { public_key, json } => {
            let mut repo = Repo::open(repo_dir)?;
            let removed = repo.remove_publisher(&public_key)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "public_key": public_key,
                        "removed": removed,
                    }))
                    .unwrap_or_default()
                );
            } else if removed {
                println!("removed {public_key}");
            } else {
                println!("not trusted: {public_key}");
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn report_invalid(error: &odc_core::Error, json: bool) -> ExitCode {
    if json {
        let report = json!({
            "valid": false,
            "error_code": error.code(),
            "error": error.to_string(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        println!("verify-signature: INVALID ({error})");
    }
    ExitCode::from(1)
}

fn write_private_key(path: &std::path::Path, contents: &str) -> odc_core::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(contents.as_bytes())?;
    set_file_mode(path, 0o600)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_survey(
    file: &Path,
    out: &Path,
    limit: usize,
    entry_selector: Option<String>,
    core_threshold: f64,
    dump_jsonl: Option<&Path>,
    dump_limit: usize,
    dump_stride: Option<usize>,
    json: bool,
) -> odc_core::Result<ExitCode> {
    let started = std::time::Instant::now();
    let source = MdxSource::open(file)?;
    let mut surveyor = Surveyor::new(SurveyOptions {
        limit: if limit == 0 { None } else { Some(limit) },
        entry_selector,
        core_threshold,
        ..SurveyOptions::default()
    })?;
    for record in source.records() {
        let record = record?;
        if !surveyor.push(&record.key, &record.html) {
            break;
        }
    }
    let elapsed = started.elapsed().as_millis();
    let report = surveyor.finish();
    if report.entries_scanned == 0 {
        eprintln!(
            "survey: no usable entry container in {} ({} records, {} alias, {} empty, {} unparsed)",
            file.display(),
            report.records,
            report.alias,
            report.empty,
            report.unparsed
        );
        return Ok(ExitCode::from(2));
    }

    std::fs::create_dir_all(out)?;
    for cluster in &report.clusters_core {
        if cluster.samples.is_empty() {
            continue;
        }
        let directory = out.join("samples").join(&cluster.id);
        std::fs::create_dir_all(&directory)?;
        for sample in &cluster.samples {
            std::fs::write(directory.join(&sample.file), &sample.html)?;
        }
    }
    let markdown = report.to_markdown(&file.display().to_string(), elapsed);
    std::fs::write(out.join("survey.md"), markdown)?;
    let json_text = serde_json::to_string_pretty(&report).map_err(|error| {
        odc_core::Error::Inconsistent(format!("survey report is not serialisable: {error}"))
    })?;
    std::fs::write(out.join("survey.json"), &json_text)?;

    if json {
        println!("{json_text}");
    } else {
        println!(
            "survey {}: {} entries, {} kinds, {} clusters (reference) -> estimate {} rules, {} ops -> {}",
            file.display(),
            report.entries_scanned,
            report.verdict.kinds,
            report.verdict.clusters,
            report.estimate.rules,
            report.estimate.ops,
            report.estimate.verdict.as_str()
        );
        println!(
            "wrote {} and {}",
            out.join("survey.md").display(),
            out.join("survey.json").display()
        );
    }
    if let Some(path) = dump_jsonl {
        write_dump(&source, path, dump_limit, dump_stride)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Write a deterministic stratified sample of body records as JSONL.
///
/// The same dictionary and the same flags always produce the same file, which
/// is what makes a dialect measurement reproducible by hand.
fn write_dump(
    source: &MdxSource,
    path: &Path,
    limit: usize,
    stride: Option<usize>,
) -> odc_core::Result<()> {
    use odc_core::survey_dump::{sample, SamplePlan, SampledRecord};

    let plan = SamplePlan {
        limit,
        stride,
        ..SamplePlan::default()
    };
    let records = || {
        source.records().map(|record| {
            record.map(|record| SampledRecord {
                key: record.key,
                text: record.html,
            })
        })
    };
    let report = sample(records, plan)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut jsonl = String::new();
    let mut keys = String::new();
    for record in &report.records {
        let line = json!({ "key": record.key, "text": record.text });
        jsonl.push_str(&line.to_string());
        jsonl.push('\n');
        keys.push_str(&record.key);
        keys.push('\n');
    }
    std::fs::write(path, jsonl)?;
    let keys_path = path.with_extension("keys");
    std::fs::write(&keys_path, keys)?;

    eprintln!(
        "dump {}: {} body records ({} alias, {} empty), kept {} -> {} + {}",
        path.display(),
        report.body,
        report.alias,
        report.empty,
        report.records.len(),
        path.display(),
        keys_path.display()
    );
    for bucket in &report.buckets {
        eprintln!(
            "  {} {}/{} of {} (stride {})",
            bucket.bucket, bucket.taken, bucket.quota, bucket.available, bucket.stride
        );
    }
    Ok(())
}

fn set_file_mode(path: &std::path::Path, mode: u32) -> odc_core::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}
