//! Entomologist adapter: reads an entomologist issue register from its orphan
//! branch through `git ls-tree` and `git cat-file --batch`, without checking out a
//! worktree, and emits a Lattice interface document.

mod gitdb;
mod issues;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Parser;
use latt_adapter::Document;
use serde::{Deserialize, Serialize};
use serde_json::ser::{CompactFormatter, Formatter};

#[derive(Debug, Parser)]
#[command(
    name = "latt-adapter-entomologist",
    about = "Read an entomologist register into a lattice interface document"
)]
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

/// The part of the resolved profile this adapter reads.
#[derive(Debug, Deserialize)]
struct ResolvedProfile {
    #[serde(default)]
    adapter: Adapter,
}

/// The one `adapter:` key this adapter consumes: a value that is not a
/// non-negative integer is refused, not read as the default.
#[derive(Debug, Default, Deserialize)]
struct Adapter {
    #[serde(default)]
    command_timeout_seconds: Option<u64>,
}

/// Check the resolved-schema marker and read the per-command bound.
fn load_profile(path: &Path) -> Result<Duration, String> {
    let profile: ResolvedProfile = latt_adapter::load_resolved_profile(path)?;
    Ok(profile
        .adapter
        .command_timeout_seconds
        .map_or(latt_adapter::DEFAULT_COMMAND_TIMEOUT, Duration::from_secs))
}

fn run(args: Args) -> Result<(), String> {
    let bound = load_profile(&args.profile)?;
    let target = latt_adapter::absolute_path(args.target).map_err(|error| error.to_string())?;
    let mut document = Document::default();
    if let Some(commit) = gitdb::resolve_commit(&mut document, &target, bound)?
        && let Some(entries) = gitdb::list_tree(&mut document, &target, &commit, bound)
    {
        issues::read(&mut document, &target, entries, bound);
    }

    let stdout = io::stdout();
    let mut writer = stdout.lock();
    let mut serializer =
        serde_json::Serializer::with_formatter(&mut writer, PythonFormatter::default());
    document
        .serialize(&mut serializer)
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::load_profile;

    /// Load a profile whose `adapter:` object is `adapter` and return its bound.
    fn command_timeout(name: &str, adapter: &str) -> Duration {
        let path = std::env::temp_dir().join(format!(
            "latt-adapter-entomologist-command-timeout-{name}-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            format!(r#"{{"resolved_schema":"1","adapter":{{{adapter}}}}}"#),
        )
        .unwrap();
        let bound = load_profile(&path);
        std::fs::remove_file(&path).unwrap();
        bound.unwrap()
    }

    #[test]
    fn command_timeout_defaults_to_60_seconds_when_absent() {
        assert_eq!(command_timeout("absent", ""), Duration::from_secs(60));
        assert_eq!(
            command_timeout("declared", r#""command_timeout_seconds":5"#),
            Duration::from_secs(5)
        );
    }
}
