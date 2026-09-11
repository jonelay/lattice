//! `lattice`: validate and query typed node/edge registers.

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use lattice_core::document::run_adapter;
use lattice_core::graph::LatticeGraph;
use lattice_core::output::{Payload, output_result, strip_ansi};
use lattice_core::profile::{Condition, Profile, load_profile, resolved_document};
use lattice_core::query::{self, Direction};
use lattice_core::suggest::render_suggestions;
use lattice_core::summary::build_summary;
use lattice_core::trace::build_trace_report;
use lattice_core::types::{Issue, Severity};
use lattice_core::validate::{resolve_adapter_issues, validate};

/// Exit 1: the register has error-severity findings.
const EXIT_FINDINGS: u8 = 1;
/// Exit 2: lattice could not run the command at all. Never collapsed into 1;
/// callers use the distinction to tell a broken setup from a real finding.
const EXIT_CONFIG: u8 = 2;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "lattice",
    about = "Validate and query typed node/edge registers over plain text.",
    // click prints `lattice, version X.Y.Z`, which clap's own flag cannot
    // spell. Handled below rather than accepting a gratuitous difference.
    disable_version_flag = true
)]
struct Cli {
    #[arg(long = "version", global = true)]
    version: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Compose multiple source registers and validate cross-source references.
    Fuse {
        /// Path to fuse manifest YAML.
        #[arg(long)]
        manifest: PathBuf,
        /// Output format (default: auto-detect).
        #[arg(long, value_parser = ["plain", "json", "rich"])]
        format: Option<String>,
        /// Promote collected warnings to errors.
        #[arg(long)]
        strict: bool,
    },
    /// Validate a register against a profile.
    Validate {
        #[command(flatten)]
        common: Common,
        /// Promote warnings to errors.
        #[arg(long)]
        strict: bool,
        /// Suggestion document to render as hints (repeatable).
        #[arg(long = "suggestions", value_name = "FILE")]
        suggestions: Vec<PathBuf>,
    },
    /// Show the computed status rollup.
    ///
    /// Takes no `--strict`: it runs no validation pass, so only error-severity
    /// adapter issues affect its exit code.
    Summary {
        #[command(flatten)]
        common: Common,
    },
    /// Show the full trace report: every node with edges and attached findings.
    Trace {
        #[command(flatten)]
        common: Common,
        /// Promote warnings to errors.
        #[arg(long)]
        strict: bool,
    },
    /// Per-kind counts of nodes with incoming and outgoing edges.
    Coverage {
        #[command(flatten)]
        common: Common,
    },
    /// Print the resolved profile document that an adapter receives.
    ///
    /// Writes to stdout so you can inspect or pipe what `--adapter` normally
    /// gets as a scratch file.
    Resolve {
        /// Path to profile YAML file.
        #[arg(long)]
        profile: PathBuf,
    },
    /// Ask the graph a question. Queries never produce findings (exit 0 or 2, never 1).
    #[command(subcommand)]
    Query(QueryCommand),
}

#[derive(Subcommand)]
enum QueryCommand {
    /// Nodes transitively reachable from a node along outgoing edges.
    Reaches {
        #[command(flatten)]
        common: Common,
        /// Origin node ID.
        id: String,
        /// Restrict traversal to this edge kind (repeatable; default: all).
        #[arg(long = "edge-kind")]
        edge_kinds: Vec<String>,
        /// Restrict returned nodes by an attribute condition (repeatable).
        #[arg(long = "filter", value_parser = query::parse_filter)]
        filters: Vec<Condition>,
        /// Mark each node tainted when every path to it crosses an undeclared endpoint.
        #[arg(long = "check-resolved")]
        check_resolved: bool,
    },
    /// Nodes that transitively reach a node along incoming edges.
    ReachedBy {
        #[command(flatten)]
        common: Common,
        /// Target node ID.
        id: String,
        /// Restrict traversal to this edge kind (repeatable; default: all).
        #[arg(long = "edge-kind")]
        edge_kinds: Vec<String>,
        /// Restrict returned nodes by an attribute condition (repeatable).
        #[arg(long = "filter", value_parser = query::parse_filter)]
        filters: Vec<Condition>,
        /// Mark each node tainted when every path to it crosses an undeclared endpoint.
        #[arg(long = "check-resolved")]
        check_resolved: bool,
    },
    /// One shortest path between two nodes, if evidence connects them.
    Path {
        #[command(flatten)]
        common: Common,
        /// Start node ID.
        src: String,
        /// End node ID.
        tgt: String,
        /// Restrict traversal to this edge kind (repeatable; default: all).
        #[arg(long = "edge-kind")]
        edge_kinds: Vec<String>,
    },
    /// Nodes with no incoming or outgoing edges.
    Orphans {
        #[command(flatten)]
        common: Common,
        /// Restrict the answer to this node kind.
        #[arg(long)]
        kind: Option<String>,
        /// Restrict returned nodes by an attribute condition (repeatable).
        #[arg(long = "filter", value_parser = query::parse_filter)]
        filters: Vec<Condition>,
    },
    /// Per-kind node and edge tallies, declared kinds shown even at zero.
    Counts {
        #[command(flatten)]
        common: Common,
        /// Restrict counted nodes by an attribute condition (repeatable).
        #[arg(long = "filter", value_parser = query::parse_filter)]
        filters: Vec<Condition>,
    },
    /// Register entries and findings originating at a source path. Findings
    /// here are informational, not a verdict - the exit code stays 0.
    At {
        #[command(flatten)]
        common: Common,
        /// Source path, absolute or target-relative; a directory matches
        /// everything beneath it.
        path: String,
        /// Restrict returned entries by a node attribute condition (repeatable).
        #[arg(long = "filter", value_parser = query::parse_filter)]
        filters: Vec<Condition>,
    },
    /// What changed between two revisions of the target. Runs the adapter
    /// at each revision and compares the results.
    Diff {
        #[command(flatten)]
        common: Common,
        /// The earlier git revision.
        rev_a: String,
        /// The later git revision.
        rev_b: String,
    },
}

impl QueryCommand {
    fn common(&self) -> &Common {
        match self {
            QueryCommand::Reaches { common, .. }
            | QueryCommand::ReachedBy { common, .. }
            | QueryCommand::Path { common, .. }
            | QueryCommand::Orphans { common, .. }
            | QueryCommand::Counts { common, .. }
            | QueryCommand::At { common, .. }
            | QueryCommand::Diff { common, .. } => common,
        }
    }
}

#[derive(Args)]
struct Common {
    /// Path to profile YAML file.
    #[arg(long)]
    profile: PathBuf,
    /// Executable adapter program emitting an interface document.
    #[arg(long)]
    adapter: PathBuf,
    /// Path to the target repo.
    #[arg(long)]
    target: PathBuf,
    /// Output format (default: auto-detect).
    #[arg(long, value_parser = ["plain", "json", "rich"])]
    format: Option<String>,
}

impl Common {
    fn format(&self) -> String {
        self.format.clone().unwrap_or_else(auto_format)
    }
}

/// `rich` for a human at a terminal, `plain` for whatever is reading the pipe.
fn auto_format() -> String {
    if std::io::stdout().is_terminal() {
        "rich".to_string()
    } else {
        "plain".to_string()
    }
}

/// Write a rendered payload the way `click.echo` does: strip ANSI when the
/// stream is not a terminal, then append the newline.
///
/// The stripping is not cosmetic: `rich` colours unconditionally, so a port
/// that wrote the rendered text straight out would differ from every captured
/// baseline on every coloured line.
fn echo(text: &str, stream: Stream) {
    let is_terminal = match stream {
        Stream::Stdout => std::io::stdout().is_terminal(),
        Stream::Stderr => std::io::stderr().is_terminal(),
    };
    let text = if is_terminal {
        text.to_string()
    } else {
        strip_ansi(text)
    };
    match stream {
        Stream::Stdout => println!("{text}"),
        Stream::Stderr => {
            let _ = writeln!(std::io::stderr(), "{text}");
        }
    }
}

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

/// Load the profile and run the adapter, or report why neither could happen.
///
/// Every failure here is operational rather than a finding about the register,
/// so each is exit 2 with a message instead of an `Issue`.
fn load(common: &Common) -> Result<(Profile, LatticeGraph), String> {
    let profile = load_profile(&common.profile).map_err(|e| format!("Profile error: {e}"))?;
    let resolved = resolved_document(&profile).map_err(|e| format!("Profile error: {e}"))?;
    let graph = run_adapter(&common.adapter, &resolved, &common.target).map_err(|e| e.0)?;
    Ok((profile, graph))
}

/// Render every named suggestion document as hint findings, in document order.
///
/// A document the core cannot use (unreadable, unparseable, or a version it
/// does not support) is a broken setup rather than a finding about the
/// register, so it travels as an `Err` the caller turns into exit 2.
fn overlay(paths: &[PathBuf], graph: &LatticeGraph) -> Result<Vec<Issue>, String> {
    let mut issues = Vec::new();
    for path in paths {
        let where_ = format!("suggestions '{}'", path.display());
        let text = std::fs::read_to_string(path).map_err(|e| format!("{where_}: {e}"))?;
        let document: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("{where_}: could not parse as JSON: {e}"))?;
        issues.extend(render_suggestions(&document, graph).map_err(|e| format!("{where_}: {e}"))?);
    }
    Ok(issues)
}

fn exit_for(has_errors: bool) -> ExitCode {
    if has_errors {
        ExitCode::from(EXIT_FINDINGS)
    } else {
        ExitCode::SUCCESS
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.version {
        println!("lattice, version {VERSION}");
        return ExitCode::SUCCESS;
    }
    let Some(command) = cli.command else {
        // clap prints help for a bare invocation; reaching here means `--version`
        // was not passed and no subcommand was named.
        let _ = writeln!(std::io::stderr(), "Error: missing command");
        return ExitCode::from(EXIT_CONFIG);
    };

    // Resolve runs no adapter and reads no register, so it answers before the
    // shared load that every other command depends on.
    if let Command::Resolve { profile } = &command {
        return match load_profile(profile).and_then(|p| resolved_document(&p)) {
            // Printed verbatim rather than through `output_result`: this is the
            // adapter handoff document, whose shape the profile-schema spec owns,
            // not a report with three renderings.
            Ok(document) => {
                println!("{document}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                let _ = writeln!(std::io::stderr(), "Error: Profile error: {e}");
                ExitCode::from(EXIT_CONFIG)
            }
        };
    }

    if let Command::Fuse {
        manifest,
        format,
        strict,
    } = &command
    {
        let format = format.clone().unwrap_or_else(auto_format);
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|binary| lattice_core::fuse::fuse(manifest, &binary, *strict));
        let report = result.unwrap_or_else(|message| lattice_core::types::FuseReport {
            findings: vec![lattice_core::types::FuseFinding {
                issue: Issue::new(
                    Severity::Error,
                    "FUSE_ERROR",
                    message,
                    lattice_core::types::Provenance::new(manifest.display().to_string(), 0),
                    None,
                ),
                source: None,
                locations: Vec::new(),
            }],
            ..Default::default()
        });
        echo(&render(&report, &format), Stream::Stdout);
        return ExitCode::from(report.exit_code());
    }

    let common = match &command {
        Command::Validate { common, .. }
        | Command::Summary { common }
        | Command::Coverage { common }
        | Command::Trace { common, .. } => common,
        Command::Query(query) => query.common(),
        Command::Resolve { .. } | Command::Fuse { .. } => unreachable!("handled above"),
    };
    let format = common.format();

    // Diff runs the adapter at two materialized revisions, not the working
    // target, so it branches off before the shared load.
    if let Command::Query(QueryCommand::Diff {
        common,
        rev_a,
        rev_b,
    }) = &command
    {
        return run_diff(common, rev_a, rev_b, &format);
    }

    let (profile, graph) = match load(common) {
        Ok(loaded) => loaded,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "Error: {message}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    match command {
        Command::Validate {
            strict,
            suggestions,
            ..
        } => {
            let mut issues = validate(&graph, &profile, strict);
            // Appended rather than merged: `output_result` sorts every finding
            // into one order, so the overlay adds to the report without
            // disturbing what the register itself found.
            match overlay(&suggestions, &graph) {
                Ok(hints) => issues.extend(hints),
                Err(message) => {
                    let _ = writeln!(std::io::stderr(), "Error: {message}");
                    return ExitCode::from(EXIT_CONFIG);
                }
            }
            let rendered = render(&issues, &format);
            // Unlike the reports, findings print nothing at all when there are
            // none rather than a bare blank line.
            if !rendered.is_empty() {
                echo(&rendered, Stream::Stdout);
            }
            exit_for(issues.iter().any(Issue::gates))
        }

        Command::Summary { .. } => {
            let adapter_issues = resolve_adapter_issues(&graph, &profile);
            let report = match build_summary(&profile, &graph) {
                Ok(report) => report,
                Err(e) => {
                    let _ = writeln!(std::io::stderr(), "Error: {e}");
                    return ExitCode::from(EXIT_CONFIG);
                }
            };
            echo(&render(&report, &format), Stream::Stdout);
            if !adapter_issues.is_empty() {
                // To stderr, so stdout stays a parseable rollup payload.
                echo(&render(&adapter_issues, &format), Stream::Stderr);
            }
            exit_for(adapter_issues.iter().any(Issue::gates))
        }

        Command::Trace { strict, .. } => {
            let issues = validate(&graph, &profile, strict);
            let has_errors = issues.iter().any(Issue::gates);
            let report = build_trace_report(&profile, &graph, issues, VERSION);
            echo(&render(&report, &format), Stream::Stdout);
            exit_for(has_errors)
        }

        // Answered on the query contract: adapter issues go to stderr and never
        // move the exit code, because a report produces no findings.
        Command::Coverage { .. } => {
            echo(
                &render(&query::coverage(&graph, &profile), &format),
                Stream::Stdout,
            );
            let adapter_issues = resolve_adapter_issues(&graph, &profile);
            if !adapter_issues.is_empty() {
                echo(&render(&adapter_issues, &format), Stream::Stderr);
            }
            ExitCode::SUCCESS
        }

        Command::Query(command) => run_query(&command, &profile, &graph, &format),

        Command::Resolve { .. } | Command::Fuse { .. } => {
            unreachable!("answered before the shared load")
        }
    }
}

/// Answer one query: payload to stdout, adapter issues to stderr, exit 0
/// or exit 2 when the question could not be posed. Never exit 1; queries
/// produce no findings, so even error-severity adapter issues only warn.
fn run_query(
    command: &QueryCommand,
    profile: &Profile,
    graph: &lattice_core::graph::LatticeGraph,
    format: &str,
) -> ExitCode {
    let kinds = |edge_kinds: &[String]| edge_kinds.iter().cloned().collect();
    let rendered = match command {
        QueryCommand::Reaches {
            id,
            edge_kinds,
            filters,
            check_resolved,
            ..
        } => query::reach(
            graph,
            profile,
            id,
            &kinds(edge_kinds),
            Direction::Forward,
            filters,
            *check_resolved,
        )
        .map(|r| render(&r, format)),
        QueryCommand::ReachedBy {
            id,
            edge_kinds,
            filters,
            check_resolved,
            ..
        } => query::reach(
            graph,
            profile,
            id,
            &kinds(edge_kinds),
            Direction::Reverse,
            filters,
            *check_resolved,
        )
        .map(|r| render(&r, format)),
        QueryCommand::Path {
            src,
            tgt,
            edge_kinds,
            ..
        } => query::path(graph, profile, src, tgt, &kinds(edge_kinds)).map(|r| render(&r, format)),
        QueryCommand::Orphans { kind, filters, .. } => {
            query::orphans(graph, profile, kind.as_deref(), filters).map(|r| render(&r, format))
        }
        QueryCommand::Counts { filters, .. } => {
            Ok(render(&query::counts(graph, profile, filters), format))
        }
        QueryCommand::At {
            common,
            path,
            filters,
        } => {
            // Never strict: findings here are answer content, not a verdict.
            let issues = validate(graph, profile, false);
            query::at(
                graph,
                profile,
                issues,
                &common.target,
                path,
                VERSION,
                filters,
            )
            .map(|r| render(&r, format))
        }
        // Handled in main before the shared load; reaching here is a wiring bug.
        QueryCommand::Diff { .. } => unreachable!("diff branches off before the shared load"),
    };

    match rendered {
        Ok(text) => {
            echo(&text, Stream::Stdout);
            let adapter_issues = resolve_adapter_issues(graph, profile);
            if !adapter_issues.is_empty() {
                echo(&render(&adapter_issues, format), Stream::Stderr);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "Error: {e}");
            ExitCode::from(EXIT_CONFIG)
        }
    }
}

/// Run the adapter at two materialized revisions and report what changed.
///
/// Each run's adapter issues go to stderr under its revision's name so a diff
/// computed from partially unreadable input at either end says so. Like every
/// query this exits 0 or 2, never 1.
fn run_diff(common: &Common, rev_a: &str, rev_b: &str, format: &str) -> ExitCode {
    let loaded = (|| -> Result<_, String> {
        let profile = load_profile(&common.profile).map_err(|e| format!("Profile error: {e}"))?;
        let resolved = resolved_document(&profile).map_err(|e| format!("Profile error: {e}"))?;
        let scratch = query::MaterializationDir::new().map_err(|e| e.0)?;
        let mut graphs = Vec::new();
        for rev in [rev_a, rev_b] {
            query::materialize_revision(&common.target, rev, &scratch).map_err(|e| e.0)?;
            graphs.push(
                run_adapter(&common.adapter, &resolved, scratch.path())
                    .map_err(|e| format!("at revision '{rev}': {}", e.0))?,
            );
        }
        let graph_b = graphs.pop().expect("two runs");
        let graph_a = graphs.pop().expect("two runs");
        Ok((profile, graph_a, graph_b))
    })();
    let (profile, graph_a, graph_b) = match loaded {
        Ok(loaded) => loaded,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "Error: {message}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    let report = query::diff(rev_a, &graph_a, rev_b, &graph_b);
    echo(&render(&report, format), Stream::Stdout);
    for (rev, graph) in [(rev_a, &graph_a), (rev_b, &graph_b)] {
        let adapter_issues = resolve_adapter_issues(graph, &profile);
        if !adapter_issues.is_empty() {
            echo(&format!("{rev}:"), Stream::Stderr);
            echo(&render(&adapter_issues, format), Stream::Stderr);
        }
    }
    ExitCode::SUCCESS
}

/// Render a payload, or die on a format the dispatcher does not know.
///
/// `--format` is constrained to the three by the parser, so an unknown one here
/// means the dispatcher and the parser have drifted apart.
fn render<'a>(payload: impl Into<Payload<'a>>, format: &str) -> String {
    output_result(payload, format).unwrap_or_else(|e| panic!("{e}"))
}
