#![deny(unsafe_code)]

//! `riftgate-replay` CLI.
//!
//! v0.3 operator surface for WAL inspection and replay workflows. The
//! current implementation provides:
//! - `dump`: decode segment frames to JSON/JSONL
//! - `replay`: replay-run summary over decoded entries
//! - `eval`: eval-run summary with eval-set TOML validation

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

const ENTRY_HEADER_BYTES: usize = 22;

#[derive(Debug, Parser)]
#[command(
    name = "riftgate-replay",
    version,
    about = "Replay tooling for Riftgate WAL segments"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decode WAL segment entries and emit structured JSON.
    Dump(DumpArgs),
    /// Replay summary over WAL entries.
    Replay(ReplayArgs),
    /// Eval summary over WAL entries + eval-set TOML.
    Eval(EvalArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DumpFormat {
    Json,
    Jsonl,
}

#[derive(Debug, Parser)]
struct DumpArgs {
    /// WAL segment paths or directories containing `*.wal` files.
    #[arg(long = "segments", required = true)]
    segments: Vec<PathBuf>,
    /// Minimum entry id (inclusive).
    #[arg(long)]
    from: Option<u64>,
    /// Maximum entry id (inclusive).
    #[arg(long)]
    to: Option<u64>,
    /// Maximum records to emit.
    #[arg(long)]
    limit: Option<usize>,
    /// Output format.
    #[arg(long, value_enum, default_value = "jsonl")]
    format: DumpFormat,
}

#[derive(Debug, Parser)]
struct ReplayArgs {
    /// WAL segment paths or directories containing `*.wal` files.
    #[arg(long = "segments", required = true)]
    segments: Vec<PathBuf>,
    /// Config file used for replay context.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Compare replay output against recorded payload bytes.
    #[arg(long, default_value_t = false)]
    compare_against_recorded: bool,
    /// Logical replay pacing multiplier.
    #[arg(long, default_value_t = 1.0)]
    rate_multiplier: f64,
}

#[derive(Debug, Parser)]
struct EvalArgs {
    /// WAL segment paths or directories containing `*.wal` files.
    #[arg(long = "segments", required = true)]
    segments: Vec<PathBuf>,
    /// Eval-set TOML path.
    #[arg(long = "eval-set")]
    eval_set: PathBuf,
    /// Stop on first eval failure.
    #[arg(long, default_value_t = false)]
    fail_fast: bool,
}

#[derive(Debug, Clone, Serialize)]
struct DecodedEntry {
    source_path: String,
    offset: u64,
    entry_id: u64,
    timestamp_nanos: u64,
    durability: String,
    entry_kind: u8,
    payload_len: u32,
    payload_base64: String,
}

#[derive(Debug, Serialize)]
struct ReplaySummary {
    run_kind: &'static str,
    entries: usize,
    bytes: u64,
    compare_against_recorded: bool,
    rate_multiplier: f64,
    config: Option<String>,
}

#[derive(Debug, Serialize)]
struct EvalSummary {
    run_kind: &'static str,
    entries: usize,
    bytes: u64,
    eval_set: String,
    fail_fast: bool,
    top_level_keys: usize,
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli) {
        eprintln!("riftgate-replay: {e}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Dump(args) => run_dump(args),
        Command::Replay(args) => run_replay(args),
        Command::Eval(args) => run_eval(args),
    }
}

fn run_dump(args: DumpArgs) -> Result<(), String> {
    let mut entries = decode_segments(&args.segments)?;
    entries.retain(|e| within_range(e.entry_id, args.from, args.to));
    if let Some(limit) = args.limit {
        entries.truncate(limit);
    }
    match args.format {
        DumpFormat::Json => {
            let out = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
            println!("{out}");
        }
        DumpFormat::Jsonl => {
            for e in &entries {
                let out = serde_json::to_string(e).map_err(|err| err.to_string())?;
                println!("{out}");
            }
        }
    }
    Ok(())
}

fn run_replay(args: ReplayArgs) -> Result<(), String> {
    let entries = decode_segments(&args.segments)?;
    let bytes: u64 = entries.iter().map(|e| u64::from(e.payload_len)).sum();
    let summary = ReplaySummary {
        run_kind: "replay",
        entries: entries.len(),
        bytes,
        compare_against_recorded: args.compare_against_recorded,
        rate_multiplier: args.rate_multiplier,
        config: args
            .config
            .as_ref()
            .map(|p| p.to_string_lossy().to_string()),
    };
    let out = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    println!("{out}");
    Ok(())
}

fn run_eval(args: EvalArgs) -> Result<(), String> {
    let entries = decode_segments(&args.segments)?;
    let bytes: u64 = entries.iter().map(|e| u64::from(e.payload_len)).sum();
    let eval_text = std::fs::read_to_string(&args.eval_set)
        .map_err(|e| format!("failed to read eval-set {}: {e}", args.eval_set.display()))?;
    let eval_value: toml::Value =
        toml::from_str(&eval_text).map_err(|e| format!("invalid eval-set TOML: {e}"))?;
    let top_level_keys = eval_value.as_table().map(|t| t.len()).unwrap_or(0);

    let summary = EvalSummary {
        run_kind: "eval",
        entries: entries.len(),
        bytes,
        eval_set: args.eval_set.to_string_lossy().to_string(),
        fail_fast: args.fail_fast,
        top_level_keys,
    };
    let out = serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?;
    println!("{out}");
    Ok(())
}

fn decode_segments(inputs: &[PathBuf]) -> Result<Vec<DecodedEntry>, String> {
    let paths = collect_segment_paths(inputs)?;
    let mut out = Vec::new();
    for p in &paths {
        decode_one_segment(p, &mut out)?;
    }
    out.sort_by_key(|e| e.entry_id);
    Ok(out)
}

fn collect_segment_paths(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for p in inputs {
        if p.is_dir() {
            let read = std::fs::read_dir(p)
                .map_err(|e| format!("failed to read directory {}: {e}", p.display()))?;
            for entry in read {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if is_wal_file(&path) {
                    out.push(path);
                }
            }
        } else if is_wal_file(p) {
            out.push(p.clone());
        } else {
            return Err(format!(
                "path is neither a .wal file nor directory: {}",
                p.display()
            ));
        }
    }
    out.sort();
    if out.is_empty() {
        return Err("no WAL segment files found".to_string());
    }
    Ok(out)
}

fn is_wal_file(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "wal")
}

fn decode_one_segment(path: &Path, out: &mut Vec<DecodedEntry>) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("open {} failed: {e}", path.display()))?;
    let mut r = BufReader::new(file);
    let mut offset: u64 = 0;

    loop {
        let mut header = [0u8; ENTRY_HEADER_BYTES];
        match read_exact_or_eof(&mut r, &mut header)
            .map_err(|e| format!("read header {} failed: {e}", path.display()))?
        {
            ReadResult::Eof => break,
            ReadResult::Read => {}
        }

        let payload_len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
        let durability = decode_durability(header[4]);
        let entry_kind = header[5];
        let timestamp_nanos = u64::from_le_bytes([
            header[6], header[7], header[8], header[9], header[10], header[11], header[12],
            header[13],
        ]);
        let entry_id = u64::from_le_bytes([
            header[14], header[15], header[16], header[17], header[18], header[19], header[20],
            header[21],
        ]);

        let mut payload = vec![0u8; payload_len as usize];
        r.read_exact(&mut payload).map_err(|e| {
            format!(
                "read payload {} failed at offset {}: {e}",
                path.display(),
                offset
            )
        })?;

        out.push(DecodedEntry {
            source_path: path.to_string_lossy().to_string(),
            offset,
            entry_id,
            timestamp_nanos,
            durability,
            entry_kind,
            payload_len,
            payload_base64: BASE64.encode(payload),
        });

        offset += ENTRY_HEADER_BYTES as u64 + u64::from(payload_len);
    }

    Ok(())
}

fn within_range(id: u64, from: Option<u64>, to: Option<u64>) -> bool {
    if let Some(f) = from
        && id < f
    {
        return false;
    }
    if let Some(t) = to
        && id > t
    {
        return false;
    }
    true
}

fn decode_durability(tag: u8) -> String {
    match tag {
        0 => "async".to_string(),
        1 => "fdatasync".to_string(),
        2 => "fsync".to_string(),
        _ => format!("unknown-{tag}"),
    }
}

enum ReadResult {
    Eof,
    Read,
}

fn read_exact_or_eof<R: Read>(r: &mut R, buf: &mut [u8]) -> io::Result<ReadResult> {
    let mut read = 0usize;
    while read < buf.len() {
        match r.read(&mut buf[read..])? {
            0 if read == 0 => return Ok(ReadResult::Eof),
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "truncated WAL header",
                ));
            }
            n => read += n,
        }
    }
    Ok(ReadResult::Read)
}
