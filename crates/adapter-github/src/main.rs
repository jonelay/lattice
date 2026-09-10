mod config;
mod gh;
mod issues;

use std::io::{self, Write};
use std::path::PathBuf;

use adapter_core::Document;
use clap::Parser;

use config::Config;
#[derive(Debug, Parser)]
#[command(about = "Read GitHub Issues into a lattice interface document")]
struct Args {
    #[arg(long)]
    profile: PathBuf,
    #[arg(long)]
    target: PathBuf,
}

fn absolute_path(path: PathBuf) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(std::fs::canonicalize(&absolute).unwrap_or(absolute))
}

fn run(args: Args) -> Result<(), String> {
    let config = Config::load(&args.profile)?;
    let target = absolute_path(args.target).map_err(|error| error.to_string())?;
    let mut document = Document::default();
    let repo = config
        .repo
        .clone()
        .or_else(|| gh::derive_repo(&mut document, &target));
    if let Some(repo) = repo
        && let Some(values) = gh::fetch_issues(&mut document, &target, &repo)
    {
        issues::read(&mut document, &config, &repo, values);
    }

    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serde_json::to_writer(&mut writer, &document)
        .map_err(|error| format!("could not serialize interface document: {error}"))?;
    writeln!(writer).map_err(|error| format!("could not write interface document: {error}"))?;
    Ok(())
}

fn main() {
    let args = Args::parse();
    if let Err(error) = run(args) {
        eprintln!("Error: {error}");
        std::process::exit(2);
    }
}
