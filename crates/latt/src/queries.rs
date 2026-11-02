//! The `query` and `diff` commands.

use std::io::Write;
use std::process::ExitCode;

use latt_core::document::{resolve_adapter, run_adapter_spec_for_profile};
use latt_core::graph::LatticeGraph;
use latt_core::output::{Format, output_result};
use latt_core::profile::{Condition, Profile, resolved_document};
use latt_core::query::{self, Direction};
use latt_core::service::{self, InputContext, InputSide, Request, View};

use super::console::{Stream, echo};
use super::loading::{
    load_profile_from, refuse_unreadable, reportable, suppress_adapter_issues, unreadable,
};
use super::{Common, Outcome, QueryCommand, describe};

/// Name, one line each, the filter attrs no node carries.
///
/// The answer stands and the exit code does not move: an attr no profile
/// declares may be one an adapter emits, so only the caller can judge it. Said
/// anyway, because a mistyped attr otherwise reads as an honest empty answer.
fn report_absent_filter_attrs(filters: &[Condition], graph: &LatticeGraph) {
    let attrs: std::collections::BTreeSet<&str> = filters.iter().map(Condition::attr).collect();
    for attr in attrs {
        if !graph.iter_nodes().any(|node| node.attrs.contains_key(attr)) {
            let _ = writeln!(
                std::io::stderr(),
                "Warning: filter attr '{attr}' appears on no node"
            );
        }
    }
}

/// Answer one query: payload to stdout, adapter issues to stderr, exit 0
/// or exit 2 when the question could not be posed. Never exit 1; queries
/// produce no findings, so even error-severity adapter issues only warn.
pub(super) fn run_query(
    command: &QueryCommand,
    profile: &Profile,
    graph: &latt_core::graph::LatticeGraph,
    format: Format,
) -> ExitCode {
    let edge_kinds = match command {
        QueryCommand::Reaches { edge_kinds, .. }
        | QueryCommand::ReachedBy { edge_kinds, .. }
        | QueryCommand::Path { edge_kinds, .. } => edge_kinds.iter().map(String::as_str).collect(),
        _ => Vec::new(),
    };
    let request = match command {
        QueryCommand::Reaches {
            id,
            filters,
            check_resolved,
            ..
        } => Request::Reach {
            origin: id,
            edge_kinds: &edge_kinds,
            direction: Direction::Forward,
            filters,
            check_resolved: *check_resolved,
        },
        QueryCommand::ReachedBy {
            id,
            filters,
            check_resolved,
            ..
        } => Request::Reach {
            origin: id,
            edge_kinds: &edge_kinds,
            direction: Direction::Reverse,
            filters,
            check_resolved: *check_resolved,
        },
        QueryCommand::Path { src, tgt, .. } => Request::Path {
            src,
            tgt,
            edge_kinds: &edge_kinds,
        },
        QueryCommand::Orphans { kind, filters, .. } => Request::Orphans {
            kind: kind.as_deref(),
            filters,
        },
        QueryCommand::Counts { filters, .. } => Request::Counts { filters },
        QueryCommand::At {
            common,
            path,
            filters,
        } => {
            let arg = path.trim_end_matches('/');
            Request::At {
                target: &common.target,
                path,
                filters,
                host_path_exists: common.target.join(arg).exists()
                    || std::path::Path::new(arg).exists(),
            }
        }
        QueryCommand::Diff { .. } => unreachable!("diff branches off before the shared load"),
    };
    let context = InputContext::default();
    match service::execute(
        View {
            graph,
            profile,
            context: &context,
        },
        request,
    ) {
        Ok(response) => {
            let mut adapter_issues: Vec<_> =
                response.diagnostics.into_iter().map(|d| d.issue).collect();
            // `at` answers with the findings at a path, so it shows the failure
            // rather than hiding it; it is the query to ask of an unreadable register.
            if !matches!(command, QueryCommand::At { .. }) && unreadable(graph, &adapter_issues) {
                return refuse_unreadable(&adapter_issues, format);
            }
            suppress_adapter_issues(&mut adapter_issues, profile, graph);
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
            report_absent_filter_attrs(command.filters(), graph);
            Outcome::Clean.into()
        }
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "Error: {}", describe(&error));
            Outcome::CouldNotRun.into()
        }
    }
}

/// Run the adapter at two materialized revisions and report what changed.
///
/// Each run's adapter issues go to stderr under its revision's name so a diff
/// computed from partially unreadable input at either end says so. A revision
/// that could not be read at all (see [`unreadable`]) is not diffed: no answer,
/// and exit 2. Like every query this exits 0 or 2.
pub(super) fn run_diff(common: &Common, rev_a: &str, rev_b: &str, format: Format) -> ExitCode {
    let loaded = (|| -> Result<_, String> {
        let profile = load_profile_from(common)?;
        let resolved =
            resolved_document(&profile).map_err(|e| format!("Profile error: {}", describe(&e)))?;
        let spec = resolve_adapter(common.adapter.as_deref().map(|p| p.as_os_str()), &profile)
            .map_err(|e| e.message().to_owned())?;
        let scratch = query::MaterializationDir::new().map_err(query::QueryError::into_message)?;
        let mut graphs = Vec::new();
        let bound = profile.adapter_timeout();
        for rev in [rev_a, rev_b] {
            query::materialize_revision(&common.target, rev, &scratch, bound)
                .map_err(query::QueryError::into_message)?;
            graphs.push(
                run_adapter_spec_for_profile(
                    &spec,
                    &resolved,
                    scratch.path(),
                    bound,
                    Some(&common.profile),
                )
                .map_err(|e| format!("at revision '{rev}': {}", e.message()))?,
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
            return Outcome::CouldNotRun.into();
        }
    };

    let context = InputContext::default();
    let response = match service::compare(
        rev_a,
        View {
            graph: &graph_a,
            profile: &profile,
            context: &context,
        },
        rev_b,
        View {
            graph: &graph_b,
            profile: &profile,
            context: &context,
        },
    ) {
        Ok(response) => response,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "Error: {}", describe(&error));
            return Outcome::CouldNotRun.into();
        }
    };
    let (before, after): (Vec<_>, Vec<_>) = response
        .diagnostics
        .into_iter()
        .partition(|diagnostic| diagnostic.side == InputSide::Before);
    let sides =
        [(rev_a, &graph_a, before), (rev_b, &graph_b, after)].map(|(rev, graph, diagnostics)| {
            let mut issues: Vec<_> = diagnostics.into_iter().map(|d| d.issue).collect();
            let unreadable = unreadable(graph, &issues);
            suppress_adapter_issues(&mut issues, &profile, graph);
            (rev, unreadable, issues)
        });
    // A revision that could not be read diffs as "everything removed" or
    // "everything added", which reads as an answer; refuse it as a query would.
    let refused = sides.iter().any(|(_, unreadable, _)| *unreadable);
    if !refused {
        echo(
            &output_result(&response.payload, format),
            format,
            Stream::Stdout,
        );
    }
    for (rev, _, adapter_issues) in &sides {
        if reportable(adapter_issues, format) {
            echo(&format!("{rev}:"), Format::Plain, Stream::Stderr);
            echo(
                &output_result(adapter_issues, format),
                format,
                Stream::Stderr,
            );
        }
    }
    if !refused {
        return Outcome::Clean.into();
    }
    for (rev, _, _) in sides.iter().filter(|(_, unreadable, _)| *unreadable) {
        let _ = writeln!(
            std::io::stderr(),
            "Error: no node could be read from the register at revision '{rev}', \
             so the diff was not answered"
        );
    }
    Outcome::CouldNotRun.into()
}
