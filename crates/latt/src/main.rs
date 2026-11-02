//! `latt`: validate and query typed node/edge registers.

#[cfg(feature = "tui")]
mod browse;
mod console;
mod loading;
mod queries;
#[cfg(feature = "tui")]
mod tui;

#[cfg(all(test, unix, feature = "tui"))]
#[path = "../tests/common/stub.rs"]
mod stub;

use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::Ordering;

use clap::{Args, Parser, Subcommand};

use latt_core::output::{Format, output_result, scope_line};
use latt_core::profile::{Condition, load_profile, resolved_document};
use latt_core::service::{self, InputContext, Report, Request, View};
use latt_core::types::{Issue, Severity};

#[cfg(feature = "tui")]
use browse::run_browse;
use console::{STDOUT_FAILED, Stream, echo, write_stdout};
use loading::{
    adapter_label, load, overlay, refuse_unreadable, reportable, suppress_adapter_issues,
    unreadable,
};
use queries::{run_diff, run_query};

/// How a command ended. The exit code is three-valued and load-bearing, so
/// the paths this binary decides name an outcome, never a number. The one
/// exception is `fuse`, whose report carries its own `exit_code()` on the
/// same three values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Exit 0: the command ran and nothing gates.
    Clean,
    /// Exit 1: the register has error-severity findings.
    Findings,
    /// Exit 2: lattice could not run the command at all. Never collapsed into
    /// 1; callers use the distinction to tell a broken setup from a real finding.
    CouldNotRun,
}

impl Outcome {
    /// The outcome of a command that ran: findings gate or they do not.
    fn gating(has_errors: bool) -> Self {
        if has_errors {
            Self::Findings
        } else {
            Self::Clean
        }
    }
}

impl From<Outcome> for ExitCode {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Clean => Self::SUCCESS,
            Outcome::Findings => Self::from(1),
            Outcome::CouldNotRun => Self::from(2),
        }
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Parser)]
#[command(
    name = "latt",
    about = "Validate and query typed node/edge registers over plain text.",
    // click prints `latt, version X.Y.Z`, which clap's own flag cannot
    // spell. Handled below rather than accepting a gratuitous difference.
    disable_version_flag = true
)]
struct Cli {
    /// Print the version and exit
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
        #[arg(long, value_parser = format_parser())]
        format: Option<Format>,
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
    /// Orient on a register: what the profile declares, what the adapter read.
    ///
    /// Each node kind with its ID pattern, count and source files; each edge kind
    /// with its allowed pairs and count; the validation entries; the adapter and its
    /// `adapter:` block. Validates nothing: exit 0 whenever it ran, 2 when it could not.
    Describe {
        #[command(flatten)]
        common: Common,
    },
    /// Print the profile document an adapter receives, resolved by the core.
    ///
    /// Writes to stdout so you can inspect or pipe what `--adapter` normally
    /// gets as a scratch file.
    Show {
        /// Path to profile YAML file.
        #[arg(long)]
        profile: PathBuf,
        /// Refuse to run unless the profile's `profile_version` is exactly this X.Y.Z.
        #[arg(long = "profile-version", value_name = "VERSION")]
        profile_version: Option<String>,
    },
    /// Run one native adapter and print the interface document it produces.
    ///
    /// For inspecting what an adapter reads, and for running a native adapter as
    /// an external program. `--profile` is the resolved profile `latt show` prints,
    /// not the profile's YAML.
    Adapter {
        /// Native adapter name: md, toml or prose-xref.
        name: String,
        /// Path to the resolved profile document (JSON).
        #[arg(long)]
        profile: PathBuf,
        /// Path to the target directory.
        #[arg(long)]
        target: PathBuf,
    },
    /// Interactively browse the loaded register in the terminal.
    ///
    /// Browse redraws the whole screen, which a screen reader cannot follow;
    /// the other commands read line by line with `--format plain`, and give
    /// structure with `--format json`.
    #[cfg(feature = "tui")]
    Browse(BrowseArgs),
    /// Ask the graph a question: exit 0 with the answer, or 2 when it could not run.
    /// An adapter's findings still print on stderr.
    #[command(subcommand)]
    Query(QueryCommand),
}

#[cfg(feature = "tui")]
#[derive(Args)]
struct BrowseArgs {
    /// Path to profile YAML file.
    #[arg(long)]
    profile: PathBuf,
    /// Refuse to run unless the profile's `profile_version` is exactly this X.Y.Z.
    #[arg(long = "profile-version", value_name = "VERSION")]
    profile_version: Option<String>,
    /// Adapter: a native name (md, toml, prose-xref), a program on PATH (entomologist from an install) or a path to one.
    #[arg(long)]
    adapter: Option<PathBuf>,
    /// Path to the target repo.
    #[arg(long)]
    target: PathBuf,
    /// Promote warnings to errors.
    #[arg(long)]
    strict: bool,
    /// Restrict nodes by an attribute condition (repeatable).
    #[arg(long = "filter", value_parser = Condition::parse)]
    filters: Vec<Condition>,
    /// Glyph set: `auto` follows the locale (Unicode when it is UTF-8);
    /// `ascii` for a terminal that draws box-drawing and arrow glyphs wide.
    #[arg(long, value_enum)]
    glyphs: Option<Glyphs>,
}

/// The glyph sets `--glyphs` chooses between.
#[cfg(feature = "tui")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Glyphs {
    Auto,
    Unicode,
    Ascii,
}

/// True when browse should draw ASCII. An explicit `--glyphs` wins; absent
/// resolves as `auto`, which follows the locale. `Option` rather than a clap
/// default keeps "not given" distinct from `auto`, so a configuration file can
/// later sit between the two.
#[cfg(feature = "tui")]
fn resolve_glyphs(choice: Option<Glyphs>, locale_is_utf8: bool) -> bool {
    match choice.unwrap_or(Glyphs::Auto) {
        Glyphs::Auto => !locale_is_utf8,
        Glyphs::Unicode => false,
        Glyphs::Ascii => true,
    }
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
        #[arg(long = "filter", value_parser = Condition::parse)]
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
        #[arg(long = "filter", value_parser = Condition::parse)]
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
    /// Nodes with neither an incoming nor an outgoing edge.
    Orphans {
        #[command(flatten)]
        common: Common,
        /// Restrict the answer to this node kind.
        #[arg(long)]
        kind: Option<String>,
        /// Restrict returned nodes by an attribute condition (repeatable).
        #[arg(long = "filter", value_parser = Condition::parse)]
        filters: Vec<Condition>,
    },
    /// Per-kind node and edge tallies, declared kinds shown even at zero.
    Counts {
        #[command(flatten)]
        common: Common,
        /// Restrict counted nodes by an attribute condition (repeatable).
        #[arg(long = "filter", value_parser = Condition::parse)]
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
        #[arg(long = "filter", value_parser = Condition::parse)]
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

    fn filters(&self) -> &[Condition] {
        match self {
            QueryCommand::Reaches { filters, .. }
            | QueryCommand::ReachedBy { filters, .. }
            | QueryCommand::Orphans { filters, .. }
            | QueryCommand::Counts { filters, .. }
            | QueryCommand::At { filters, .. } => filters,
            QueryCommand::Path { .. } | QueryCommand::Diff { .. } => &[],
        }
    }
}

#[derive(Args)]
struct Common {
    /// Path to profile YAML file.
    #[arg(long)]
    profile: PathBuf,
    /// Refuse to run unless the profile's `profile_version` is exactly this X.Y.Z.
    #[arg(long = "profile-version", value_name = "VERSION")]
    profile_version: Option<String>,
    /// Adapter: a native name (md, toml, prose-xref), a program on PATH (entomologist from an install) or a path to one.
    #[arg(long)]
    adapter: Option<PathBuf>,
    /// Path to the target repo.
    #[arg(long)]
    target: PathBuf,
    /// Output format (default: auto-detect).
    #[arg(long, value_parser = format_parser())]
    format: Option<Format>,
}

impl Common {
    fn format(&self) -> Format {
        self.format.unwrap_or_else(auto_format)
    }
}

/// Parse `--format` against the library's spellings. The possible-values list
/// keeps `--help` and clap's error for a bad value as they were under the
/// `ValueEnum` derive, so `FromStr` only ever sees a valid spelling.
fn format_parser() -> impl clap::builder::TypedValueParser<Value = Format> {
    use clap::builder::TypedValueParser;
    clap::builder::PossibleValuesParser::new(Format::ALL.iter().map(|format| format.as_str()))
        .try_map(|value| value.parse::<Format>())
}

/// `rich` for a human at a terminal, `plain` for whatever is reading the pipe.
fn auto_format() -> Format {
    if std::io::stdout().is_terminal() {
        Format::Rich
    } else {
        Format::Plain
    }
}

/// The error and every cause beneath it, one per indented line, so a wrapped
/// I/O or parse failure reaches the user instead of stopping at the wrapper.
fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut cause = error.source();
    while let Some(next) = cause {
        text.push_str("\n  caused by: ");
        text.push_str(&next.to_string());
        cause = next.source();
    }
    text
}

fn exit_for(has_errors: bool) -> ExitCode {
    Outcome::gating(has_errors).into()
}

fn main() -> ExitCode {
    let code = run();
    if STDOUT_FAILED.load(Ordering::Relaxed) {
        Outcome::CouldNotRun.into()
    } else {
        code
    }
}

fn run() -> ExitCode {
    let cli = Cli::parse();
    if cli.version {
        write_stdout(&format!("latt, version {VERSION}"));
        return Outcome::Clean.into();
    }
    let Some(command) = cli.command else {
        // clap prints help for a bare invocation; reaching here means `--version`
        // was not passed and no subcommand was named.
        let _ = writeln!(std::io::stderr(), "Error: missing command");
        return Outcome::CouldNotRun.into();
    };

    // Show runs no adapter and reads no register, so it answers before the
    // shared load that every other command depends on.
    if let Command::Show {
        profile,
        profile_version,
    } = &command
    {
        let loaded = load_profile(profile).and_then(|p| {
            if let Some(exact) = profile_version {
                p.require_version(exact)?;
            }
            resolved_document(&p)
        });
        return match loaded {
            // Printed verbatim rather than through `output_result`: this is the
            // adapter handoff document, whose shape the profile-schema spec owns,
            // not a report with three renderings.
            Ok(document) => {
                write_stdout(&document);
                Outcome::Clean.into()
            }
            Err(e) => {
                let _ = writeln!(std::io::stderr(), "Error: Profile error: {}", describe(&e));
                Outcome::CouldNotRun.into()
            }
        };
    }

    // The adapter writes its own document; nothing is ingested or validated.
    if let Command::Adapter {
        name,
        profile,
        target,
    } = &command
    {
        let mut out = std::io::BufWriter::new(std::io::stdout().lock());
        return match latt_core::document::write_native_document(name, profile, target, &mut out) {
            Ok(()) => Outcome::Clean.into(),
            Err(message) => {
                let _ = writeln!(std::io::stderr(), "Error: {message}");
                Outcome::CouldNotRun.into()
            }
        };
    }

    if let Command::Fuse {
        manifest,
        format,
        strict,
    } = &command
    {
        let format = format.unwrap_or_else(auto_format);
        let result = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|binary| {
                latt_core::fuse::fuse(manifest, &binary, *strict).map_err(|e| describe(&e))
            });
        let report = result.unwrap_or_else(|message| {
            latt_core::types::FuseReport::failed(Issue::new(
                Severity::Error,
                latt_core::fuse::FUSE_ERROR,
                message,
                latt_core::types::Provenance::new_file(manifest.display().to_string()),
                None,
            ))
        });
        echo(&output_result(&report, format), format, Stream::Stdout);
        return ExitCode::from(report.exit_code());
    }

    #[cfg(feature = "tui")]
    if let Command::Browse(args) = command {
        return run_browse(args);
    }

    let common = match &command {
        Command::Validate { common, .. }
        | Command::Summary { common }
        | Command::Coverage { common }
        | Command::Describe { common }
        | Command::Trace { common, .. } => common,
        Command::Query(query) => query.common(),
        Command::Show { .. } | Command::Fuse { .. } | Command::Adapter { .. } => {
            unreachable!("handled above")
        }
        #[cfg(feature = "tui")]
        Command::Browse(_) => unreachable!("handled above"),
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
        return run_diff(common, rev_a, rev_b, format);
    }

    let (profile, graph) = match load(common) {
        Ok(loaded) => loaded,
        Err(message) => {
            let _ = writeln!(std::io::stderr(), "Error: {message}");
            return Outcome::CouldNotRun.into();
        }
    };

    if let Command::Describe { common } = &command {
        let adapter = match adapter_label(common, &profile) {
            Ok(label) => label,
            Err(message) => {
                let _ = writeln!(std::io::stderr(), "Error: {message}");
                return Outcome::CouldNotRun.into();
            }
        };
        let report = match latt_core::describe::build_describe_report(&profile, &graph, adapter) {
            Ok(report) => report,
            Err(e) => {
                let _ = writeln!(std::io::stderr(), "Profile error: {}", describe(&e));
                return Outcome::CouldNotRun.into();
            }
        };
        echo(&output_result(&report, format), format, Stream::Stdout);
        // An orientation view, not an audit: the adapter's findings are shown,
        // and gate nothing.
        return Outcome::Clean.into();
    }

    let request = match &command {
        Command::Validate { strict, .. } => Request::Validate { strict: *strict },
        Command::Summary { .. } => Request::Summary,
        Command::Trace { strict, .. } => Request::Trace { strict: *strict },
        Command::Coverage { .. } => Request::Coverage,
        Command::Query(command) => return run_query(command, &profile, &graph, format),
        Command::Show { .. }
        | Command::Fuse { .. }
        | Command::Adapter { .. }
        | Command::Describe { .. } => {
            unreachable!("handled above")
        }
        #[cfg(feature = "tui")]
        Command::Browse(_) => unreachable!("handled above"),
    };
    let context = InputContext::default();
    let response = match service::execute(
        View {
            graph: &graph,
            profile: &profile,
            context: &context,
        },
        request,
    ) {
        Ok(response) => response,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "Error: {}", describe(&error));
            return Outcome::CouldNotRun.into();
        }
    };
    match command {
        Command::Validate { suggestions, .. } => {
            let Report::Validate(mut report) = response.payload else {
                unreachable!("validate response")
            };
            // Suggestion files remain CLI I/O; hints append after service validation.
            match overlay(&suggestions, &graph) {
                Ok(hints) => report.findings.extend(hints),
                Err(message) => {
                    let _ = writeln!(std::io::stderr(), "Error: {message}");
                    return Outcome::CouldNotRun.into();
                }
            }
            let rendered = output_result(&report, format);
            if !rendered.is_empty() {
                echo(&rendered, format, Stream::Stdout);
            }
            // JSON carries the scope as a key; the text formats say it on
            // stderr, so a clean run's stdout stays empty for CI.
            if format != Format::Json {
                echo(&scope_line(&report.scope), format, Stream::Stderr);
            }
            exit_for(report.findings.iter().any(Issue::gates))
        }
        Command::Trace { .. } => {
            let Report::Trace(report) = response.payload else {
                unreachable!("trace response")
            };
            let has_errors = report
                .entries
                .iter()
                .flat_map(|entry| &entry.findings)
                .chain(&report.unattachable_findings)
                .any(Issue::gates);
            echo(&output_result(&report, format), format, Stream::Stdout);
            exit_for(has_errors)
        }
        Command::Summary { .. } | Command::Coverage { .. } => {
            let mut adapter_issues: Vec<_> =
                response.diagnostics.into_iter().map(|d| d.issue).collect();
            if matches!(command, Command::Coverage { .. }) && unreadable(&graph, &adapter_issues) {
                return refuse_unreadable(&adapter_issues, format);
            }
            suppress_adapter_issues(&mut adapter_issues, &profile, &graph);
            echo(
                &output_result(&response.payload, format),
                format,
                Stream::Stdout,
            );
            if reportable(&adapter_issues, format) {
                echo(
                    &output_result(&adapter_issues, format),
                    format,
                    Stream::Stderr,
                );
            }
            // Summary gates on the adapter issues the profile left unsuppressed,
            // as validate does; coverage is a query.
            exit_for(
                matches!(command, Command::Summary { .. })
                    && adapter_issues.iter().any(Issue::gates),
            )
        }
        _ => unreachable!("handled above"),
    }
}

/// The command table in `docs/cli.md` is the CLI's public index; a subcommand it
/// omits, or lists without existing, is drift a reader hits first.
#[cfg(test)]
mod cli_doc_command_table {
    use clap::CommandFactory;

    use super::Cli;

    fn cli_doc() -> String {
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cli.md")).unwrap()
    }

    /// Every leaf subcommand as `sub` or `sub child`.
    fn leaves() -> Vec<(String, clap::Command)> {
        let mut paths = Vec::new();
        for sub in Cli::command().get_subcommands() {
            if sub.has_subcommands() {
                for child in sub.get_subcommands() {
                    let path = format!("{} {}", sub.get_name(), child.get_name());
                    paths.push((path, child.clone()));
                }
            } else {
                paths.push((sub.get_name().to_owned(), sub.clone()));
            }
        }
        paths.sort_by(|a, b| a.0.cmp(&b.0));
        paths
    }

    /// The first column of the first table under Commands in `docs/cli.md`,
    /// `a/b` alternatives split into one entry each; `browse` dropped when it is
    /// not compiled in. Later tables in the section (filter operators) are not
    /// commands.
    fn documented_commands() -> Vec<String> {
        let text = cli_doc();
        let table = text
            .split("## Commands")
            .nth(1)
            .expect("docs/cli.md has a Commands section");
        let mut names = Vec::new();
        for line in table
            .lines()
            .skip_while(|line| !line.starts_with('|'))
            .take_while(|line| line.starts_with('|'))
            .filter(|line| line.starts_with("| `"))
        {
            let cell = line.trim_start_matches("| `").split('`').next().unwrap();
            match cell.split_once(' ') {
                Some((head, alternatives)) => {
                    names.extend(alternatives.split('/').map(|alt| format!("{head} {alt}")));
                }
                None => names.push(cell.to_owned()),
            }
        }
        if !cfg!(feature = "tui") {
            names.retain(|name| name != "browse");
        }
        names.sort();
        names
    }

    // Requirement: development-verification/Verify documentation and supported feature configurations
    #[test]
    fn the_command_table_lists_every_subcommand_and_nothing_else() {
        let cli: Vec<String> = leaves().into_iter().map(|(path, _)| path).collect();
        assert_eq!(documented_commands(), cli);
    }

    // Requirement: development-verification/Verify documentation and supported feature configurations
    #[test]
    fn format_is_on_exactly_the_commands_the_cli_doc_says() {
        let text = cli_doc();
        let sentence = text
            .lines()
            .find(|line| line.starts_with("All commands except"))
            .expect("docs/cli.md says which commands take --format");
        let exceptions: Vec<&str> = sentence
            .split('`')
            .skip(1)
            .step_by(2)
            .take_while(|word| !word.starts_with("--format"))
            .collect();
        assert!(!exceptions.is_empty(), "the sentence names its exceptions");
        for (path, command) in leaves() {
            let has_format = command
                .get_arguments()
                .any(|arg| arg.get_long() == Some("format"));
            assert_eq!(
                has_format,
                !exceptions.contains(&path.as_str()),
                "{path}: --format presence disagrees with docs/cli.md"
            );
        }
        for exception in exceptions {
            let compiled_out = exception == "browse" && !cfg!(feature = "tui");
            assert!(
                compiled_out || leaves().iter().any(|(path, _)| path == exception),
                "{exception}: docs/cli.md excepts a command that does not exist"
            );
        }
    }
}

#[cfg(all(test, feature = "tui"))]
mod glyph_mode_tests {
    use super::{Glyphs, resolve_glyphs};

    #[test]
    fn the_flag_beats_the_locale_in_both_directions() {
        assert!(resolve_glyphs(Some(Glyphs::Ascii), true));
        assert!(!resolve_glyphs(Some(Glyphs::Unicode), false));
        assert!(resolve_glyphs(Some(Glyphs::Auto), false));
        assert!(!resolve_glyphs(Some(Glyphs::Auto), true));
        // Absent means auto.
        assert!(resolve_glyphs(None, false));
        assert!(!resolve_glyphs(None, true));
    }
}

#[cfg(all(test, feature = "tui"))]
mod glyph_flag_tests {
    use clap::Parser;
    use clap::error::ErrorKind;

    use super::{Cli, Command, Glyphs};

    fn parsed(extra: &[&str]) -> Result<Option<Glyphs>, clap::Error> {
        let mut argv = vec!["latt", "browse", "--profile", "p.yaml", "--target", "."];
        argv.extend_from_slice(extra);
        Cli::try_parse_from(argv).map(|cli| match cli.command {
            Some(Command::Browse(args)) => args.glyphs,
            _ => panic!("parsed as another subcommand"),
        })
    }

    #[test]
    fn glyphs_takes_three_values_and_may_be_absent() {
        assert_eq!(parsed(&[]).unwrap(), None);
        assert_eq!(parsed(&["--glyphs", "auto"]).unwrap(), Some(Glyphs::Auto));
        assert_eq!(
            parsed(&["--glyphs", "unicode"]).unwrap(),
            Some(Glyphs::Unicode)
        );
        assert_eq!(parsed(&["--glyphs", "ascii"]).unwrap(), Some(Glyphs::Ascii));
    }

    #[test]
    fn an_unknown_glyph_set_is_an_invalid_value_naming_the_three() {
        let error = parsed(&["--glyphs", "wide"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidValue);
        let text = error.to_string();
        for value in ["auto", "unicode", "ascii"] {
            assert!(text.contains(value), "{text}");
        }
    }
}
