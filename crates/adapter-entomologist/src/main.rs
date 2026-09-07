mod document;
mod gitdb;
mod issues;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Parser;
use serde::Serialize;
use serde_json::ser::{CompactFormatter, Formatter};

use document::Document;

#[derive(Debug, Parser)]
#[command(about = "Read an entomologist register into a lattice contract document")]
struct Args {
    #[arg(long)]
    profile: PathBuf,
    #[arg(long)]
    target: PathBuf,
}

#[derive(Debug, Default)]
struct PythonFormatter(CompactFormatter);

impl Formatter for PythonFormatter {
    fn begin_array_value<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        if !first {
            writer.write_all(b", ")?;
        }
        Ok(())
    }

    fn begin_object_key<W>(&mut self, writer: &mut W, first: bool) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        if !first {
            writer.write_all(b", ")?;
        }
        Ok(())
    }

    fn begin_object_value<W>(&mut self, writer: &mut W) -> io::Result<()>
    where
        W: ?Sized + Write,
    {
        writer.write_all(b": ")
    }
}

fn absolute_path(path: PathBuf) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(std::fs::canonicalize(&absolute).unwrap_or(absolute))
}

fn validate_profile(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
    let profile: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("could not load profile {}: {error}", path.display()))?;
    if profile
        .get("resolved_schema")
        .and_then(|value| value.as_str())
        != Some("1")
    {
        return Err(format!(
            "could not load profile {}: not a resolved profile document",
            path.display()
        ));
    }
    Ok(())
}

fn run(args: Args) -> Result<(), String> {
    validate_profile(&args.profile)?;
    let target = absolute_path(args.target).map_err(|error| error.to_string())?;
    let mut document = Document::default();
    if let Some(commit) = gitdb::resolve_commit(&mut document, &target)
        && let Some(entries) = gitdb::list_tree(&mut document, &target, &commit)
    {
        issues::read(&mut document, &target, entries);
    }

    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut writer, PythonFormatter::default());
    document
        .serialize(&mut serializer)
        .map_err(|error| format!("could not serialize contract document: {error}"))?;
    writeln!(writer).map_err(|error| format!("could not write contract document: {error}"))?;
    Ok(())
}

fn main() {
    let args = Args::parse();
    if let Err(error) = run(args) {
        eprintln!("Error: {error}");
        std::process::exit(2);
    }
}
