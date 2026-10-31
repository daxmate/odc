//! `odc` command line interface (M0).

use clap::{Parser, Subcommand};
use odc_core::cbor;
use odc_core::entry::{normalize_key, Def, Entry};
use odc_core::hash;
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
        } => {
            let mut repo = Repo::open(&cli.repo)?;
            if format == "mdx" {
                return import_mdx(&mut repo, &file, prune, limit, progress);
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
            Ok(ExitCode::SUCCESS)
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
                    Ok(ExitCode::SUCCESS)
                }
                None => Err(odc_core::Error::InvalidArgument(format!(
                    "entry not found: {}",
                    normalize_key(&word)
                ))),
            }
        }
        Command::Lookup { word, json } => {
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
                    Ok(ExitCode::SUCCESS)
                }
                None => {
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
                "index: {} entries, {} blocks",
                status.entries, status.blocks
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Patch { command } => {
            let mut repo = Repo::open(&cli.repo)?;
            match command {
                PatchCommand::Create { base, target } => {
                    let plan = patch::create(&repo, &base, &target)?;
                    println!("patch plan with {} blocks", plan.new_blocks.len());
                    Ok(ExitCode::SUCCESS)
                }
                PatchCommand::Apply { file } => {
                    let bytes = std::fs::read(&file)?;
                    patch::apply(&mut repo, &bytes)?;
                    Ok(ExitCode::SUCCESS)
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

fn import_mdx(
    repo: &mut Repo,
    path: &std::path::Path,
    prune: bool,
    limit: Option<usize>,
    progress: bool,
) -> odc_core::Result<ExitCode> {
    use odc_core::mdx::{entry_from_record, MdxKeys, MdxSource};

    let source = MdxSource::open(path)?;
    let header = source.header();
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

    let mut records = source.records();
    let mut keys = MdxKeys::new();
    let mut taken = 0usize;
    let mut skipped_empty = 0usize;
    let mut skipped_oversized = 0usize;
    let mut deduplicated = 0usize;
    let mut next_mark = 10_000usize;

    let report = repo.import_stream(
        || loop {
            if limit.is_some_and(|cap| taken >= cap) {
                return Ok(None);
            }
            let Some(record) = records.next() else {
                return Ok(None);
            };
            let record = record?;
            if record.key.trim().is_empty() {
                skipped_empty += 1;
                continue;
            }
            let mut entry = match entry_from_record(&record) {
                Ok(entry) => entry,
                Err(odc_core::Error::EntryTooLarge { .. }) => {
                    skipped_oversized += 1;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let (key, collided) = keys.allocate(&entry.word);
            entry.word = key;
            if collided {
                deduplicated += 1;
            }
            taken += 1;
            return Ok(Some(entry));
        },
        prune,
        |written| {
            if progress && written >= next_mark {
                eprintln!("progress: {written} entries");
                next_mark = written + 10_000;
            }
        },
    )?;

    println!(
        "imported {} entries ({} written, {} removed); working set = {}",
        report.written, report.written, report.removed, report.total
    );
    if skipped_empty + skipped_oversized + deduplicated > 0 {
        println!(
            "mdx: {deduplicated} duplicate keys suffixed, {skipped_empty} empty skipped, {skipped_oversized} oversized skipped"
        );
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
