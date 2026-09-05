use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use epub_stream_cleaner::{clean_batch, clean_epub, CleanOptions};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "epub-clean", version, about = "High-fidelity streaming EPUB cleaner")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Clean one EPUB and atomically write a validated output archive.
    Clean(CleanArgs),
    /// Clean all .epub files in a directory with Rayon parallelism.
    Batch(BatchArgs),
}

#[derive(Debug, Args, Clone)]
struct PolicyArgs {
    /// Keep external HTTP(S) anchor links instead of unwrapping them.
    #[arg(long)]
    keep_external_links: bool,
    /// Keep containers whose id/class contains an exact ad marker.
    #[arg(long)]
    keep_ad_containers: bool,
    /// Maximum decompressed size accepted for a single ZIP entry.
    #[arg(long, default_value_t = 256)]
    max_entry_mib: u64,
}

impl PolicyArgs {
    fn options(&self) -> CleanOptions {
        CleanOptions {
            remove_external_links: !self.keep_external_links,
            remove_ad_containers: !self.keep_ad_containers,
            max_entry_bytes: self.max_entry_mib.saturating_mul(1024 * 1024),
        }
    }
}

#[derive(Debug, Args)]
struct CleanArgs {
    input: PathBuf,
    output: PathBuf,
    #[command(flatten)]
    policy: PolicyArgs,
    /// Write the JSON report to this path.
    #[arg(long)]
    report: Option<PathBuf>,
}

#[derive(Debug, Args)]
struct BatchArgs {
    input_dir: PathBuf,
    output_dir: PathBuf,
    #[command(flatten)]
    policy: PolicyArgs,
    /// Write a JSON array report to this path.
    #[arg(long)]
    report: Option<PathBuf>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Clean(args) => {
            let report = clean_epub(&args.input, &args.output, &args.policy.options())?;
            print_report(&report, args.report.as_deref())?;
        }
        Command::Batch(args) => {
            let reports = clean_batch(&args.input_dir, &args.output_dir, &args.policy.options())?;
            let json = serde_json::to_string_pretty(&reports)?;
            if let Some(path) = args.report {
                fs::write(&path, format!("{json}\n"))
                    .with_context(|| format!("write report {}", path.display()))?;
            } else {
                println!("{json}");
            }
        }
    }
    Ok(())
}

fn print_report<T: serde::Serialize>(report: &T, path: Option<&std::path::Path>) -> Result<()> {
    let json = serde_json::to_string_pretty(report)?;
    if let Some(path) = path {
        fs::write(path, format!("{json}\n"))
            .with_context(|| format!("write report {}", path.display()))?;
    } else {
        println!("{json}");
    }
    Ok(())
}
