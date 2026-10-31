//! `odc` command line interface (M0).

use clap::{Parser, Subcommand};
use odc_core::cbor;
use odc_core::entry::{normalize_key, Def, Entry};
use odc_core::hash;
use odc_core::patch;
use odc_core::repo::Repo;
use serde_json::json;
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
        /// Input file (JSONL or CSV).
        file: PathBuf,
        /// Input format.
        #[arg(long, value_parser = ["jsonl", "csv"], default_value = "jsonl")]
        format: String,
        /// Delete working-set entries that are absent from the input file.
        #[arg(long)]
        prune: bool,
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
    /// Check every block, entry and commit.
    Verify,
    /// Show the working set relative to HEAD.
    Status,
    /// Patch creation and application (specified, implemented in M3).
    Patch {
        #[command(subcommand)]
        command: PatchCommand,
    },
}

#[derive(Subcommand, Debug)]
enum PatchCommand {
    /// Create a patch between two revisions.
    Create { base: String, target: String },
    /// Apply a patch file.
    Apply { file: PathBuf },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> odc_core::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => {
            let repo = Repo::init(&cli.repo)?;
            let head = repo
                .head()?
                .map_or_else(|| "".to_string(), |hash| hash::to_hex(&hash));
            println!("initialized ODC repository at {}", cli.repo.display());
            println!("genesis {}", &head[..12.min(head.len())]);
            Ok(())
        }
        Command::Import {
            file,
            format,
            prune,
        } => {
            let mut repo = Repo::open(&cli.repo)?;
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
            Ok(())
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
            Ok(())
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
            Ok(())
        }
        Command::Show { word, json } => {
            let repo = Repo::open(&cli.repo)?;
            match repo.show(&word)? {
                Some(entry) => {
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&entry_json(&entry)).unwrap_or_default()
                        );
                    } else {
                        print_entry(&entry);
                    }
                    Ok(())
                }
                None => Err(odc_core::Error::InvalidArgument(format!(
                    "entry not found: {}",
                    normalize_key(&word)
                ))),
            }
        }
        Command::Diff { from, to, json } => {
            let repo = Repo::open(&cli.repo)?;
            let diff = repo.diff(&from, &to)?;
            if json {
                let value = json!({
                    "from": diff.from,
                    "to": diff.to,
                    "added": diff.added,
                    "removed": diff.removed,
                    "modified": diff.modified,
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&value).unwrap_or_default()
                );
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
            Ok(())
        }
        Command::Verify => {
            let repo = Repo::open(&cli.repo)?;
            let report = repo.verify()?;
            if report.ok {
                println!(
                    "verify: OK ({} blocks, {} entries, {} commits)",
                    report.blocks, report.entries, report.commits
                );
                Ok(())
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
            Ok(())
        }
        Command::Patch { command } => {
            let mut repo = Repo::open(&cli.repo)?;
            match command {
                PatchCommand::Create { base, target } => {
                    let plan = patch::create(&repo, &base, &target)?;
                    println!("patch plan with {} blocks", plan.new_blocks.len());
                    Ok(())
                }
                PatchCommand::Apply { file } => {
                    let bytes = std::fs::read(&file)?;
                    patch::apply(&mut repo, &bytes)
                }
            }
        }
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
