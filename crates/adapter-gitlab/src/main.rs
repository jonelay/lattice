mod config;
mod glab;
mod issues;

use std::io::{self, Write};
use std::path::PathBuf;

use adapter_core::Document;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Read GitLab issues into a lattice interface document")]
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
    let config = config::load(&args.profile)?;
    let target = absolute_path(args.target).map_err(|error| error.to_string())?;
    let mut document = Document::default();
    let project = match config.project.clone() {
        Some(project) => Some(project),
        None => match glab::discover_project(&target) {
            Ok(project) => Some(project),
            Err(error) => {
                document.parse_error(error, ".", 0);
                None
            }
        },
    };

    if let Some(project) = project {
        match glab::issues(&project) {
            Ok(gitlab_issues) => {
                issues::read(&mut document, &project, &config, gitlab_issues, |iid| {
                    glab::links(&project, iid)
                })
            }
            Err(error) => document.parse_error(error, format!("gitlab:{project}"), 0),
        }
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
