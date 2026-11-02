//! Native adapter benchmarks: each adapter writes its interface document from a
//! register generated once, outside timing, into the bench's target scratch
//! directory, and the timed routine is `write_native_document` as `run_native`
//! calls it.
//!
//! The boundary is warm-filesystem, not cold I/O: the register and the resolved
//! profile were just written and stay in the page cache. Each iteration includes
//! loading and decoding the resolved profile file, walking and reading the
//! register, the adapter's parse and mapping, and serializing the document into a
//! fresh `Vec<u8>` that grows from empty and is dropped inside the iteration, as
//! `run_native` allocates and drops it. Ingesting that document is not included;
//! `document/ingest-text` times it.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput};
use latt_core::document::{ingest_text, write_native_document};
use latt_core::profile::{load_profile_value, resolved_document};
use serde_json::{Value, json};

use super::Digest;
use super::operations::{OPERATIONS, Operation, ScaleSet, ThroughputBasis};

pub struct AdapterScale {
    pub label: String,
    adapter: String,
    generator: String,
    params: Value,
    nodes: usize,
    edges: usize,
    pub expected: Option<Value>,
}

pub struct AdapterFixture {
    pub scale: AdapterScale,
    profile: PathBuf,
    target: PathBuf,
}

/// The scratch directory the registers are generated into, removed when the
/// harness ends. The process ID keeps two concurrent runs apart.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new() -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("adapter-registers-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).expect("a stale scratch directory is removable");
        }
        fs::create_dir_all(&root).expect("scratch directory is creatable");
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn param(scale: &AdapterScale, name: &str) -> usize {
    scale.params[name]
        .as_u64()
        .unwrap_or_else(|| panic!("{}: generator parameter {name}", scale.label)) as usize
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().expect("a register file has a parent"))
        .expect("register directory is creatable");
    fs::write(path, text).expect("register file is writable");
}

/// Cadence at which the TOML register writes a `tags` array holding a table entry:
/// one `UNREAD_ENTRIES` warning per row so written.
const TOML_UNREAD_EVERY: usize = 1024;
const TOML_UNREAD_AT: usize = 512;

/// `rows` `[[item]]` rows over `files` files under `registers/`, in ID order. Each row
/// maps five keys, a long string among them and a list attr, links to the next row by
/// a scalar edge key and cites the two rows before it by an array edge key.
fn toml_register(target: &Path, rows: usize, files: usize) {
    let per_file = rows.div_ceil(files);
    for file in 0..files {
        let mut text = format!("# Synthetic TOML register, file {file}.\n\n");
        for index in file * per_file..((file + 1) * per_file).min(rows) {
            let _ = writeln!(text, "[[item]]");
            let _ = writeln!(text, "id = \"n{index:05}\"");
            let _ = writeln!(
                text,
                "title = \"Item {index}: a requirement long enough to carry a sentence of prose\""
            );
            let _ = writeln!(text, "status = \"active\"");
            let _ = writeln!(text, "ordinal = {index}");
            if index % TOML_UNREAD_EVERY == TOML_UNREAD_AT {
                let _ = writeln!(text, "tags = [\"t{}\", {{ glob = \"*.md\" }}]", index % 7);
            } else {
                let _ = writeln!(text, "tags = [\"t{}\", \"u{}\"]", index % 7, index % 11);
            }
            if index + 1 < rows {
                let _ = writeln!(text, "next = \"n{:05}\"", index + 1);
            }
            let cites: Vec<String> = [2, 3]
                .into_iter()
                .filter(|back| index >= *back)
                .map(|back| format!("\"n{:05}\"", index - back))
                .collect();
            if !cites.is_empty() {
                let _ = writeln!(text, "cites = [{}]", cites.join(", "));
            }
            text.push('\n');
        }
        write(&target.join(format!("registers/r{file:03}.toml")), &text);
    }
}

const TOML_PROFILE: &str = r#"
name: synthetic-toml-v1
profile_version: "1.0.0"
adapter:
  paths:
    files: ["registers/*.toml"]
  tables:
    item:
      kind: item
      id_key: id
      key_map: { id: id, title: title, status: status, ordinal: ordinal, tags: tags }
      edge_keys: { next: next, cites: cites }
node_kinds:
  item:
    id_pattern: '^n[0-9]{5}$'
    attrs:
      id: { type: string, required: true }
      title: { type: string, required: true }
      status: { type: enum, values: [active] }
      ordinal: { type: int }
      tags: { type: list, items: string }
edge_kinds:
  next:
    allowed: [[item, item]]
  cites:
    allowed: [[item, item]]
"#;

/// `rows` table rows split evenly over `files` files and `tables` tables in each.
/// Every description holds an escaped pipe, every row traces to the two rows before it
/// in one comma-separated cell, and every 64th row adds a link target whose comma does
/// not separate. Each file opens with a fenced example table, which is not read, and
/// ends with one short row: a `PARSE_ERROR` and no node.
fn md_register(target: &Path, rows: usize, files: usize, tables: usize) {
    let per_table = rows.div_ceil(files * tables);
    let mut index = 0;
    for file in 0..files {
        let mut text = format!(
            "# Register {file}\n\nProse before the tables is not read.\n\n\
             ```text\n| ID | Description | Status | Traces To |\n| --- | --- | --- | --- |\n\
             | REQ-0 | an example, not a row | draft | |\n```\n"
        );
        for table in 0..tables {
            let _ = write!(
                text,
                "\n## Table {table}\n\n| ID | Description | Status | Traces To |\n| --- | --- | --- | --- |\n"
            );
            for _ in 0..per_table {
                if index == rows {
                    break;
                }
                let mut traces: Vec<String> = [1, 2]
                    .into_iter()
                    .filter(|back| index >= *back)
                    .map(|back| format!("REQ-{}", index - back))
                    .collect();
                if index >= 2 && index.is_multiple_of(64) {
                    traces.push(format!("[REQ-{}, REQ-{}](#r)", index - 1, index - 2));
                }
                let _ = writeln!(
                    text,
                    "| REQ-{index} | Requirement {index} keeps `a \\| b` apart | active | {} |",
                    traces.join(", ")
                );
                index += 1;
            }
        }
        text.push_str("| REQ-SHORT | too few cells |\n");
        write(&target.join(format!("reqs/f{file:03}.md")), &text);
    }
}

const MD_PROFILE: &str = r#"
name: synthetic-md-v1
profile_version: "1.0.0"
adapter:
  paths:
    files: ["reqs/*.md"]
  table:
    id_column: "ID"
    column_map:
      "Description": "summary"
      "Status": "status"
    edge_columns:
      "Traces To": "traces_to"
node_kinds:
  requirement:
    id_pattern: "^REQ-[0-9]+$"
    attrs:
      summary: { type: string, required: true }
      status: { type: string }
edge_kinds:
  traces_to:
    allowed: [[requirement, requirement]]
"#;

/// `files` documents alternating between two content directories, each with `sections`
/// level-2 headings. Each section cites a heading in the next file and, carrying that
/// file forward on the same line, a second one; on its own line it cites a heading in
/// its own file. A `§` in inline code is on every section's line too and is not a
/// reference; every eighth section holds a fenced block with a heading and a reference,
/// neither read; each file's last line is a bare `§`, a `PARSE_ERROR`.
fn prose_register(target: &Path, files: usize, sections: usize) {
    let path = |file: usize| format!("{}/f{file:03}.md", ["notes", "outline"][file % 2]);
    for file in 0..files {
        let other = path((file + 1) % files);
        let mut text = format!("# File {file}\n\nAn opening paragraph with no reference.\n");
        for section in 0..sections {
            let _ = write!(
                text,
                "\n## Section {section}\n\nSection {section} reads on in `{other}` § `Section {}` \
                 and § `Section {}` for context.\nBack to § `Section {}` here; the marker `§` \
                 in code is not a reference.\n",
                (section + 1) % sections,
                (section + 2) % sections,
                (section + sections - 1) % sections,
            );
            if section % 8 == 4 {
                text.push_str(
                    "\n```text\n## Not a heading\nSee § `Section 0` inside a fence.\n```\n",
                );
            }
        }
        text.push_str("\nA bare § Reference here.\n");
        write(&target.join(path(file)), &text);
    }
}

const PROSE_PROFILE: &str = r#"
name: synthetic-prose-xref-v1
profile_version: "1.0.0"
adapter:
  paths:
    files: ["**/*.md"]
    content_dirs: ["notes", "outline"]
node_kinds:
  document:
    id_pattern: "^.+\\.md$"
    orphan_ok: true
  section:
    id_pattern: "^[^#]+\\.md#.+$"
    orphan_ok: true
    attrs:
      heading: { type: string, required: true }
      level: { type: int }
edge_kinds:
  xref:
    allowed:
      - [section, section]
      - [document, section]
"#;

fn scales(manifest: &Value) -> Vec<AdapterScale> {
    manifest["adapter_scales"]
        .as_array()
        .expect("adapter scales are an array")
        .iter()
        .map(|scale| AdapterScale {
            label: scale["label"].as_str().expect("scale label").to_owned(),
            adapter: scale["adapter"].as_str().expect("scale adapter").to_owned(),
            generator: scale["generator"]
                .as_str()
                .expect("scale generator")
                .to_owned(),
            params: scale["parameters"].clone(),
            nodes: scale["nodes"].as_u64().expect("scale node count") as usize,
            edges: scale["edges"].as_u64().expect("scale edge count") as usize,
            expected: scale.get("expected").cloned(),
        })
        .collect()
}

/// Generate every adapter scale's register and resolved profile under `scratch`.
/// A label shared with a graph scale, or with another adapter scale, refuses here,
/// since one measurement ID would then name two runs.
pub fn fixtures(manifest: &Value, scratch: &Scratch) -> Vec<AdapterFixture> {
    let graph_labels = manifest["scales"]
        .as_array()
        .expect("fixture scales are an array")
        .iter()
        .map(|scale| scale["label"].as_str().expect("scale label"));
    let mut seen = std::collections::BTreeSet::new();
    let scales = scales(manifest);
    for label in graph_labels.chain(scales.iter().map(|scale| scale.label.as_str())) {
        assert!(
            seen.insert(label.to_owned()),
            "scale label {label} is not unique"
        );
    }
    scales
        .into_iter()
        .map(|scale| {
            let root = scratch.0.join(&scale.label);
            let target = root.join("register");
            let profile_yaml = match scale.generator.as_str() {
                "synthetic-toml-v1" => {
                    toml_register(&target, param(&scale, "rows"), param(&scale, "files"));
                    TOML_PROFILE
                }
                "synthetic-md-v1" => {
                    md_register(
                        &target,
                        param(&scale, "rows"),
                        param(&scale, "files"),
                        param(&scale, "tables"),
                    );
                    MD_PROFILE
                }
                "synthetic-prose-xref-v1" => {
                    prose_register(&target, param(&scale, "files"), param(&scale, "sections"));
                    PROSE_PROFILE
                }
                other => panic!("scale {} names unknown generator {other}", scale.label),
            };
            let profile = load_profile_value(
                serde_norway::from_str(profile_yaml).expect("synthetic profile YAML parses"),
                &scale.generator,
            )
            .expect("synthetic profile loads");
            let profile_path = root.join("profile.json");
            fs::write(
                &profile_path,
                resolved_document(&profile).expect("synthetic profile resolves"),
            )
            .expect("resolved profile is writable");
            AdapterFixture {
                scale,
                profile: profile_path,
                target,
            }
        })
        .collect()
}

fn document(fixture: &AdapterFixture) -> Vec<u8> {
    let mut out = Vec::new();
    write_native_document(
        &fixture.scale.adapter,
        &fixture.profile,
        &fixture.target,
        &mut out,
    )
    .unwrap_or_else(|error| panic!("{}: adapter fails: {error}", fixture.scale.label));
    out
}

/// Compute the pinned results through the routine the benchmark times, then ingest the
/// document and check the graph against the scale's declared counts. The document must
/// not name the scratch directory, or its digest would differ from run to run.
pub fn computed(fixture: &AdapterFixture) -> Value {
    let label = &fixture.scale.label;
    let bytes = document(fixture);
    let text = std::str::from_utf8(&bytes).expect("the document is UTF-8");
    assert!(
        !text.contains(&*fixture.target.to_string_lossy()),
        "{label}: the document names the scratch directory"
    );
    let graph = ingest_text(text).expect("the adapter's document ingests");
    let nodes = graph.iter_nodes().count();
    let edges = graph.iter_edges().count();
    assert_eq!(nodes, fixture.scale.nodes, "{label}: stored nodes");
    assert_eq!(edges, fixture.scale.edges, "{label}: stored edges");
    let mut codes = BTreeMap::<&str, u64>::new();
    for issue in graph.adapter_issues() {
        *codes.entry(issue.code.as_str()).or_default() += 1;
    }
    let mut digest = Digest::new();
    digest.bytes(&bytes);
    json!({
        "document": { "bytes": bytes.len(), "fnv1a64": digest.hex() },
        "nodes": nodes,
        "edges": edges,
        "findings": codes
    })
}

/// The declared operation that times `fixture`'s adapter; a scale whose adapter no
/// operation names, or an operation the list does not declare, fails at bench time.
fn operation(fixture: &AdapterFixture) -> &'static Operation {
    OPERATIONS
        .iter()
        .find(|operation| {
            matches!(operation.scales, ScaleSet::Adapter(name) if name == fixture.scale.adapter)
        })
        .unwrap_or_else(|| {
            panic!(
                "{}: no declared operation runs adapter {}",
                fixture.scale.label, fixture.scale.adapter
            )
        })
}

pub fn benchmark(c: &mut Criterion, fixtures: &[AdapterFixture]) {
    let mut group = c.benchmark_group("adapter");
    for fixture in fixtures {
        let operation = operation(fixture);
        assert_eq!(operation.group, "adapter");
        if operation.throughput == Some(ThroughputBasis::Nodes) {
            group.throughput(Throughput::Elements(fixture.scale.nodes as u64));
        }
        group.bench_with_input(
            BenchmarkId::new(operation.function, &fixture.scale.label),
            fixture,
            |b, fixture| {
                // body: adapter/toml adapter/md adapter/prose-xref
                b.iter(|| {
                    let mut out = Vec::new();
                    write_native_document(
                        &fixture.scale.adapter,
                        black_box(&fixture.profile),
                        black_box(&fixture.target),
                        &mut out,
                    )
                    .expect("the adapter writes its document");
                    black_box(out)
                });
                // body-end
            },
        );
    }
    group.finish();
}
