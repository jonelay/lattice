#![allow(dead_code)]

//! Shared fixture loading: shipped profiles, their registers, and a query runner.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use latt_core::document::{ingest_document, resolve_adapter, run_adapter_spec};
use latt_core::graph::LatticeGraph;
use latt_core::profile::{Profile, load_profile, resolved_document};
use latt_trustfall::adapter::LatticeAdapter;
use trustfall_core::frontend::parse;
use trustfall_core::interpreter::execution::interpret_ir;
use trustfall_core::ir::FieldValue;
use trustfall_core::schema::Schema;

pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every shipped profile under `profiles/`, by file name.
pub fn shipped_profiles() -> Vec<(String, Profile)> {
    let mut out: Vec<_> = std::fs::read_dir(root().join("profiles"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "yaml"))
        .map(|path| {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            (name, load_profile(&path).expect("a shipped profile loads"))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The shipped profiles whose fixture register a test can read in-process: the
/// native adapters over their `mini-*` fixtures, and the synthetic trace
/// baseline's document. The entomologist and `openspec` profiles need a git
/// register or a Python adapter the Python suite provides, and are the
/// generator's to cover.
pub fn fixture_pairs() -> Vec<(String, Profile, LatticeGraph)> {
    let native = [
        ("md", "md", "tests/fixtures/mini-md"),
        ("md-multi", "md", "tests/fixtures/mini-md-multi"),
        ("md-sources", "md", "tests/fixtures/mini-md-sources"),
        ("prose-xref", "prose-xref", "tests/fixtures/mini-prose-xref"),
        ("toml", "toml", "tests/fixtures/mini-toml"),
        (
            "toml-directory",
            "toml",
            "tests/fixtures/mini-toml-directory",
        ),
    ];
    let mut pairs = Vec::new();
    for (name, adapter, target) in native {
        let profile = load_profile(&root().join(format!("profiles/{name}.yaml"))).unwrap();
        let resolved = resolved_document(&profile).unwrap();
        let spec = resolve_adapter(Some(std::ffi::OsStr::new(adapter)), &profile)
            .expect("a native adapter");
        let graph = run_adapter_spec(
            &spec,
            &resolved,
            &root().join(target),
            profile.adapter_timeout(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        pairs.push((name.to_string(), profile, graph));
    }
    let fixtures = root().join("crates/latt-core/tests/fixtures");
    let profile = load_profile(&fixtures.join("synthetic.profile.yaml")).unwrap();
    let document: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fixtures.join("synthetic.document.json")).unwrap(),
    )
    .unwrap();
    pairs.push((
        "synthetic".into(),
        profile,
        ingest_document(document).unwrap(),
    ));
    pairs
}

/// Run one query with no arguments; rows as output name → value.
pub fn run(
    schema: &Schema,
    adapter: &Arc<LatticeAdapter<'_>>,
    query: &str,
) -> Vec<BTreeMap<Arc<str>, FieldValue>> {
    let indexed = parse(schema, query).unwrap_or_else(|e| panic!("{query}\n{e}"));
    interpret_ir(Arc::clone(adapter), indexed, Arc::new(BTreeMap::new()))
        .unwrap_or_else(|e| panic!("{query}\n{e}"))
        .collect()
}
