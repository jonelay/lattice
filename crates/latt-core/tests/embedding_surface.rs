//! The supported Rust surface, named by path.
//!
//! Each module below imports every supported module-level item of one public
//! module of `latt-core` or `latt-adapter`, so removing or moving one fails
//! to compile under whichever feature set the test runs with. Methods, fields
//! and variants cannot be named by `use`; the per-file counts below and the
//! semver gate (`tools/verify-rust-tooling.sh`) cover them. The "Supported
//! items" list in `docs/embedding.md` must name exactly these items.
// Requirement: graph-service/Document the supported embedding surface
#![allow(unused_imports)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

mod latt_core {
    use latt_core::serde_norway;
}

mod latt_core_document {
    use latt_core::document::{
        AdapterSpec, ContractError, INTERFACE_VERSION, NativeAdapter, ResolveError,
        SUPPORTED_INTERFACE_VERSIONS, ingest_document, ingest_text, parse_document,
        resolve_adapter, run_adapter_spec,
    };
}

mod latt_core_graph {
    use latt_core::graph::{
        DuplicateNode, Edge, EdgeSpec, LatticeGraph, Node, Pathway, PathwayError, check_pathway,
    };
}

mod latt_core_output {
    use latt_core::output::{Format, ParseFormatError, Payload, output_result};
}

mod latt_core_profile {
    use latt_core::profile::{
        AttrSchema, AttrType, Comparable, Condition, EdgeKind, FilterError, NodeKind,
        PathwayBinding, Profile, ProfileError, load_profile, load_profile_value, resolved_document,
    };
}

mod latt_core_service {
    use latt_core::service::{
        AtReport, CivilDate, CountsReport, CoverageReport, DiffReport, Direction, EdgeChangedRef,
        EdgeRef, ErrorKind, ExecutionStatus, FindingTally, InputContext, InputDiagnostic,
        InputRecord, InputSide, InputStatus, Issue, KindCoverage, NodeChangedRef, NodeRef, Note,
        Origin, OrphanEntry, OrphansReport, PathReport, PathwayEntry, Provenance, ReachReport,
        Report, Request, Response, Scope, ServiceError, Severity, StatusRollup, StructuralSummary,
        SummaryReport, TraceEdge, TraceEntry, TraceReport, Until, ValidateReport, ValidateScope,
        View, compare, execute, prepare_profile, prepare_profile_str,
    };
}

mod latt_core_suggest {
    use latt_core::suggest::{SUGGESTION_VERSION, SUPPORTED_SUGGESTION_VERSIONS};
}

mod latt_core_validate {
    use latt_core::validate::{apply_messages, apply_suppressions};
}

mod latt_adapter {
    use latt_adapter::{
        Document, Edge, INTERFACE_VERSION, InvalidLine, Issue, Node, Pathway, Provenance, Severity,
        decode_lines, decode_utf8,
    };
}

mod latt_adapter_codes {
    use latt_adapter::codes::{
        ALL, AMBIGUOUS_REF, EXTERNAL_REF, NO_MATCHING_FILES, PARSE_ERROR, PATHWAY_INVALID,
        SOURCE_MISSING, UNREAD_ENTRIES, WRAPPED_REF,
    };
}

mod latt_adapter_fence {
    use latt_adapter::fence::FenceState;
}

/// Parse `use <path>::{a, b};` and `use <path>::a;` statements into a module map.
fn uses(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for stmt in text.split("use ").skip(1) {
        let Some((stmt, _)) = stmt.split_once(';') else {
            continue;
        };
        let stmt: String = stmt.split_whitespace().collect();
        if !stmt.starts_with("latt_core") && !stmt.starts_with("latt_adapter") {
            continue;
        }
        let (path, names) = match stmt.split_once("::{") {
            Some((path, names)) => (path, names.trim_end_matches('}')),
            None => stmt.rsplit_once("::").expect("a path"),
        };
        let entry = map.entry(path.to_string()).or_default();
        entry.extend(
            names
                .split(',')
                .filter(|n| !n.is_empty())
                .map(str::to_string),
        );
        entry.sort();
    }
    map
}

/// Parse the "Supported items" list: one ``- `path`: `a`, `b` `` line per module.
fn documented(markdown: &str) -> BTreeMap<String, Vec<String>> {
    let section = markdown
        .split("### Supported items")
        .nth(1)
        .and_then(|rest| rest.split("\n#").next())
        .expect("docs/embedding.md has a Supported items section");
    let mut map = BTreeMap::new();
    for line in section.lines().filter(|l| l.starts_with("- `")) {
        let (path, names) = line[2..].split_once(": ").expect("- `path`: items");
        let mut names: Vec<String> = names
            .split(", ")
            .map(|n| n.trim_matches('`').to_string())
            .collect();
        names.sort();
        map.insert(path.trim_matches('`').to_string(), names);
    }
    map
}

#[test]
fn the_documented_supported_items_are_the_witnessed_ones() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let docs = fs::read_to_string(root.join("docs/embedding.md")).unwrap();
    assert_eq!(
        documented(&docs),
        uses(include_str!("embedding_surface.rs"))
    );
}

/// Count `pub` declarations and fields in each library source file, split into
/// visible and hidden by a `#[doc(hidden)]` on the line directly above.
///
/// A textual tripwire, not the boundary: an accidental `pub` changes a number
/// here, and the review of that number is the point. `fuse/` counts as
/// visible text although `lib.rs` hides its whole module.
fn pub_counts() -> BTreeMap<String, (usize, usize)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["crates/latt-core/src", "crates/latt-adapter/src"] {
        collect(&root.join(dir), &mut files);
    }
    let mut counts = BTreeMap::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let mut previous = "";
        let (mut visible, mut hidden) = (0, 0);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with("pub ") {
                if previous == "#[doc(hidden)]" {
                    hidden += 1;
                } else {
                    visible += 1;
                }
            }
            previous = line;
        }
        let name = file
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        counts.insert(name, (visible, hidden));
    }
    counts
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "bin") {
                collect(&path, files);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
}

/// `(file, visible, hidden)` per library source file.
const PINNED: &[(&str, usize, usize)] = &[
    ("crates/latt-adapter/src/codes.rs", 9, 0),
    ("crates/latt-adapter/src/fence.rs", 4, 0),
    ("crates/latt-adapter/src/files.rs", 0, 6),
    ("crates/latt-adapter/src/lib.rs", 61, 4),
    ("crates/latt-adapter/src/process.rs", 9, 4),
    ("crates/latt-adapter/src/profile.rs", 0, 3),
    ("crates/latt-adapter/src/sorted_map.rs", 9, 0),
    ("crates/latt-adapter/src/sources/go.rs", 0, 0),
    ("crates/latt-adapter/src/sources/mod.rs", 0, 2),
    ("crates/latt-adapter/src/sources/python.rs", 0, 0),
    ("crates/latt-adapter/src/sources/rust.rs", 0, 0),
    ("crates/latt-adapter/src/sources/tests.rs", 0, 0),
    // Under the hidden `adapters` module: unstable, whatever the per-line count says.
    ("crates/latt-core/src/adapters/md/config.rs", 0, 0),
    ("crates/latt-core/src/adapters/md/mod.rs", 2, 0),
    ("crates/latt-core/src/adapters/md/parse.rs", 0, 0),
    ("crates/latt-core/src/adapters/mod.rs", 3, 0),
    ("crates/latt-core/src/adapters/prose_xref/config.rs", 0, 0),
    ("crates/latt-core/src/adapters/prose_xref/mod.rs", 4, 0),
    ("crates/latt-core/src/adapters/prose_xref/parse.rs", 0, 0),
    ("crates/latt-core/src/adapters/toml/config.rs", 0, 0),
    ("crates/latt-core/src/adapters/toml/mod.rs", 2, 0),
    ("crates/latt-core/src/adapters/toml/tree.rs", 0, 0),
    ("crates/latt-core/src/algo.rs", 5, 0),
    ("crates/latt-core/src/describe.rs", 0, 1),
    ("crates/latt-core/src/document.rs", 14, 3),
    ("crates/latt-core/src/document/wire.rs", 0, 0),
    ("crates/latt-core/src/fuse/compose.rs", 1, 0),
    ("crates/latt-core/src/fuse/fuse_profile.rs", 2, 0),
    ("crates/latt-core/src/fuse/manifest.rs", 1, 0),
    ("crates/latt-core/src/fuse/mod.rs", 22, 0),
    ("crates/latt-core/src/fuse/trace.rs", 2, 0),
    ("crates/latt-core/src/graph.rs", 45, 0),
    ("crates/latt-core/src/lib.rs", 8, 9),
    ("crates/latt-core/src/notes.rs", 0, 0),
    ("crates/latt-core/src/output/describe.rs", 0, 0),
    ("crates/latt-core/src/output/fuse.rs", 0, 0),
    ("crates/latt-core/src/output/mod.rs", 7, 2),
    ("crates/latt-core/src/output/query.rs", 0, 0),
    ("crates/latt-core/src/output/sorted.rs", 0, 0),
    ("crates/latt-core/src/output/summary.rs", 0, 0),
    ("crates/latt-core/src/output/trace.rs", 8, 0),
    ("crates/latt-core/src/output/validate.rs", 1, 0),
    ("crates/latt-core/src/profile/compile.rs", 0, 0),
    ("crates/latt-core/src/profile/conditions.rs", 0, 0),
    ("crates/latt-core/src/profile/kinds.rs", 0, 0),
    ("crates/latt-core/src/profile/mod.rs", 5, 4),
    ("crates/latt-core/src/profile/raw.rs", 1, 0),
    ("crates/latt-core/src/profile/types.rs", 104, 26),
    ("crates/latt-core/src/query/components.rs", 0, 3),
    ("crates/latt-core/src/query/index.rs", 4, 5),
    ("crates/latt-core/src/query/layout.rs", 0, 1),
    ("crates/latt-core/src/query/layout/tests.rs", 0, 0),
    ("crates/latt-core/src/query/materialize.rs", 2, 2),
    ("crates/latt-core/src/query/mod.rs", 14, 0),
    ("crates/latt-core/src/query/reports.rs", 6, 0),
    ("crates/latt-core/src/query/schema.rs", 1, 1),
    ("crates/latt-core/src/query/traverse.rs", 1, 1),
    ("crates/latt-core/src/scope.rs", 0, 0),
    ("crates/latt-core/src/service.rs", 42, 3),
    ("crates/latt-core/src/suggest.rs", 2, 1),
    ("crates/latt-core/src/summary.rs", 5, 1),
    ("crates/latt-core/src/template.rs", 10, 2),
    ("crates/latt-core/src/trace.rs", 1, 0),
    ("crates/latt-core/src/types.rs", 234, 26),
    ("crates/latt-core/src/validate/adapter_issues.rs", 1, 0),
    ("crates/latt-core/src/validate/collect.rs", 0, 0),
    ("crates/latt-core/src/validate/conditions.rs", 0, 1),
    ("crates/latt-core/src/validate/coverage.rs", 5, 0),
    ("crates/latt-core/src/validate/mod.rs", 3, 6),
    ("crates/latt-core/src/validate/tests.rs", 0, 0),
];

#[test]
fn pub_declaration_counts_are_pinned() {
    let actual = pub_counts();
    let pinned: BTreeMap<String, (usize, usize)> = PINNED
        .iter()
        .map(|&(file, visible, hidden)| (file.to_string(), (visible, hidden)))
        .collect();
    let table: String = actual
        .iter()
        .map(|(file, (v, h))| format!("    (\"{file}\", {v}, {h}),\n"))
        .collect();
    assert_eq!(pinned, actual, "update PINNED to:\n{table}");
}
