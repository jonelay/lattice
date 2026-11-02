//! Stable component benchmarks for Lattice's development performance history.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::Duration;

use criterion::measurement::WallTime;
use criterion::{BatchSize, BenchmarkGroup, BenchmarkId, Criterion, Throughput};
use latt_core::document::{ingest_document, ingest_text, parse_document};
use latt_core::output::{Format, output_result};
use latt_core::query::{self, Direction};
use latt_core::service::{self, InputContext, Report, Request, Response, ServiceError, View};
use latt_core::types::CountsReport;
use latt_core::validate::validate;
use serde_json::{Value, json};

mod operations;
use operations::{OPERATIONS, ThroughputBasis};

mod adapters;

mod pins;
use pins::Pinned;

const MANIFEST: &str = include_str!("fixture.json");
const PROFILE: &str = r#"
name: synthetic-chain-v1
profile_version: "1.0.0"
node_kinds:
  item:
    id_pattern: '^N[0-9]+$'
    attrs:
      ordinal: { type: int, required: true }
      status: { type: enum, required: true, values: [active] }
edge_kinds:
  next:
    allowed: [[item, item]]
"#;

struct Scale {
    label: String,
    generator: String,
    chain: Option<usize>,
    nodes: usize,
    edges: usize,
    expected: Option<Value>,
}

struct Fixture {
    scale: Scale,
    document: String,
    graph: latt_core::graph::LatticeGraph,
    profile: latt_core::profile::Profile,
    issues: Vec<latt_core::types::Issue>,
}

fn manifest() -> Value {
    serde_json::from_str(MANIFEST).expect("benchmark fixture manifest is valid JSON")
}

fn scales(manifest: &Value) -> Vec<Scale> {
    let scales: Vec<Scale> = manifest["scales"]
        .as_array()
        .expect("fixture scales are an array")
        .iter()
        .map(|scale| Scale {
            label: scale["label"].as_str().expect("scale label").to_owned(),
            generator: scale["generator"]
                .as_str()
                .expect("scale generator")
                .to_owned(),
            chain: scale
                .get("chain_nodes")
                .map(|chain| chain.as_u64().expect("chain length") as usize),
            nodes: scale["nodes"].as_u64().expect("scale node count") as usize,
            edges: scale["edges"].as_u64().expect("scale edge count") as usize,
            expected: scale.get("expected").cloned(),
        })
        .collect();
    for (index, scale) in scales.iter().enumerate() {
        assert!(
            scales[..index].iter().all(|seen| seen.label != scale.label),
            "fixture scale label {} is declared twice",
            scale.label
        );
    }
    scales
}

fn item(index: usize, missing_every: usize, file: &str, line: usize) -> Value {
    let mut attrs =
        serde_json::Map::from_iter([("status".to_owned(), Value::String("active".to_owned()))]);
    if !index.is_multiple_of(missing_every) {
        attrs.insert("ordinal".to_owned(), json!(index));
    }
    json!({
        "id": format!("N{index}"),
        "kind": "item",
        "attrs": attrs,
        "provenance": { "file": file, "line": line }
    })
}

fn edge(src: String, tgt: String, kind: &str, file: &str, line: usize) -> Value {
    json!({
        "src": src,
        "tgt": tgt,
        "kind": kind,
        "attrs": {},
        "provenance": { "file": file, "line": line }
    })
}

fn chain_document(scale: &Scale, missing_every: usize) -> String {
    const FILE: &str = "synthetic-chain-v1.json";
    assert_eq!(scale.edges, scale.nodes.saturating_sub(1));
    let nodes: Vec<Value> = (0..scale.nodes)
        .map(|index| item(index, missing_every, FILE, index + 1))
        .collect();
    let edges: Vec<Value> = (0..scale.edges)
        .map(|index| {
            let line = scale.nodes + index + 1;
            edge(
                format!("N{index}"),
                format!("N{}", index + 1),
                "next",
                FILE,
                line,
            )
        })
        .collect();
    serde_json::to_string(&json!({
        "interface_version": "1.2",
        "nodes": nodes,
        "edges": edges,
        "findings": []
    }))
    .expect("synthetic document serializes")
}

/// The chain as a native adapter writes it: every attr has its own line below its
/// node's, and every edge names the node entry that declared it. Per-attr locations
/// are what raised 0.8.0's ingestion cost, which the other generators never carried.
fn located_document(chain: usize, missing_every: usize) -> String {
    const FILE: &str = "synthetic-located-v1.toml";
    let nodes: Vec<Value> = (0..chain)
        .map(|index| {
            let line = 3 * index + 1;
            let mut node = item(index, missing_every, FILE, line);
            let attrs = node["attrs"].as_object().expect("an item has attrs");
            let locations: serde_json::Map<String, Value> = ["ordinal", "status"]
                .into_iter()
                .zip(1..)
                .filter(|(attr, _)| attrs.contains_key(*attr))
                .map(|(attr, offset)| {
                    (
                        attr.to_owned(),
                        json!({ "file": FILE, "line": line + offset }),
                    )
                })
                .collect();
            node["attr_provenance"] = Value::Object(locations);
            node
        })
        .collect();
    let edges: Vec<Value> = (0..chain.saturating_sub(1))
        .map(|index| {
            let mut edge = edge(
                format!("N{index}"),
                format!("N{}", index + 1),
                "next",
                FILE,
                3 * chain + index + 1,
            );
            edge["declared_by"] = json!(index);
            edge
        })
        .collect();
    serde_json::to_string(&json!({
        "interface_version": "1.2",
        "nodes": nodes,
        "edges": edges,
        "findings": []
    }))
    .expect("synthetic document serializes")
}

/// The chain plus one instance of each defect class per cadence window, so the
/// error paths validation and reach take are timed and pinned beside the clean ones.
/// `X` nodes are numbered past the chain and reachable only through an undeclared
/// `U` endpoint; duplicates are appended, so the first declaration still wins.
fn defects_document(chain: usize, missing_every: usize) -> String {
    const FILE: &str = "synthetic-defects-v1.json";
    let mut line = 0;
    let mut next_line = || {
        line += 1;
        line
    };
    let mut nodes: Vec<Value> = (0..chain)
        .map(|index| item(index, missing_every, FILE, next_line()))
        .collect();
    let mut edges = Vec::new();
    let mut findings = Vec::new();
    for index in 0..chain {
        let here = format!("N{index}");
        if index + 1 < chain {
            edges.push(edge(
                here.clone(),
                format!("N{}", index + 1),
                "next",
                FILE,
                next_line(),
            ));
        }
        match index % 128 {
            5 | 69 => {
                nodes.push(json!({
                    "id": format!("Q{index}"), "kind": "ghost", "attrs": {},
                    "provenance": { "file": FILE, "line": next_line() }
                }));
                edges.push(edge(
                    here.clone(),
                    format!("Q{index}"),
                    "next",
                    FILE,
                    next_line(),
                ));
            }
            9 | 73 if index + 1 < chain => {
                edges.push(edge(
                    here.clone(),
                    format!("N{}", index + 1),
                    "jumps",
                    FILE,
                    next_line(),
                ));
            }
            13 | 77 => edges.push(edge(
                here.clone(),
                format!("M{index}"),
                "next",
                FILE,
                next_line(),
            )),
            17 | 81 => edges.push(edge(
                format!("S{index}"),
                here.clone(),
                "next",
                FILE,
                next_line(),
            )),
            21 => {
                let x = 1_000_000 + index;
                nodes.push(item(x, missing_every, FILE, next_line()));
                edges.push(edge(
                    here.clone(),
                    format!("U{index}"),
                    "next",
                    FILE,
                    next_line(),
                ));
                edges.push(edge(
                    format!("U{index}"),
                    format!("N{x}"),
                    "next",
                    FILE,
                    next_line(),
                ));
            }
            25 | 89 if index + 1 < chain => {
                edges.push(edge(
                    here.clone(),
                    format!("N{}", index + 1),
                    "next",
                    FILE,
                    next_line(),
                ));
            }
            29 => nodes.push(item(index, missing_every, FILE, next_line())),
            33 => findings.push(json!({
                "severity": "warning",
                "code": "ADAPTER_REPORTED",
                "message": format!("register row for {here} was ambiguous"),
                "provenance": { "file": FILE, "line": next_line() },
                "node_id": here
            })),
            _ => {}
        }
    }
    serde_json::to_string(&json!({
        "interface_version": "1.2",
        "nodes": nodes,
        "edges": edges,
        "findings": findings
    }))
    .expect("synthetic document serializes")
}

/// Cadences of the rules register, shared by its document, its profile and the check
/// of its findings, so what each entry produces and what `SUPPRESS` hides follow from them.
const RULES_RETIRED_EVERY: usize = 8;
const RULES_FORK_EVERY: usize = 16;
const RULES_FORK_AT: usize = 3;
const RULES_CITE_EVERY: usize = 4;

/// A chain whose profile carries `EDGE_WHERE`, `EDGE_COUNT` and `SUPPRESS` entries, so
/// the per-kind rule lookup, the ungrouped count and the suppression index are timed.
/// Every eighth node is `retired`; a node at 3 mod 16 forks a second `next` two ahead;
/// every fourth node `cites` the node two ahead with a `weight` cycling 0, 1, 2.
fn rules_document(chain: usize) -> String {
    const FILE: &str = "synthetic-rules-v1.json";
    let nodes: Vec<Value> = (0..chain)
        .map(|index| {
            let status = if index.is_multiple_of(RULES_RETIRED_EVERY) {
                "retired"
            } else {
                "active"
            };
            json!({
                "id": format!("N{index}"),
                "kind": "item",
                "attrs": { "ordinal": index, "status": status },
                "provenance": { "file": FILE, "line": index + 1 }
            })
        })
        .collect();
    let mut line = chain;
    let mut next_line = || {
        line += 1;
        line
    };
    let mut edges = Vec::new();
    for index in 0..chain {
        let here = format!("N{index}");
        if index + 1 < chain {
            edges.push(edge(
                here.clone(),
                format!("N{}", index + 1),
                "next",
                FILE,
                next_line(),
            ));
        }
        if index + 2 < chain {
            if index % RULES_FORK_EVERY == RULES_FORK_AT {
                edges.push(edge(
                    here.clone(),
                    format!("N{}", index + 2),
                    "next",
                    FILE,
                    next_line(),
                ));
            }
            if index.is_multiple_of(RULES_CITE_EVERY) {
                let mut cite = edge(
                    here.clone(),
                    format!("N{}", index + 2),
                    "cites",
                    FILE,
                    next_line(),
                );
                cite["attrs"] = json!({ "weight": (index / RULES_CITE_EVERY) % 3 });
                edges.push(cite);
            }
        }
    }
    serde_json::to_string(&json!({
        "interface_version": "1.2",
        "nodes": nodes,
        "edges": edges,
        "findings": []
    }))
    .expect("synthetic document serializes")
}

/// The ID of the source of each `next` edge into a retired node below half the chain:
/// the findings the `node_ids` `SUPPRESS` entry hides.
fn rules_suppressed(chain: usize) -> Vec<String> {
    (RULES_RETIRED_EVERY..chain / 2)
        .step_by(RULES_RETIRED_EVERY)
        .map(|retired| format!("N{}", retired - 1))
        .collect()
}

/// The rules register's profile: three `EDGE_WHERE` entries over two edge kinds, two
/// ungrouped `EDGE_COUNT` entries, and `SUPPRESS` both code-wide and by node list.
fn rules_profile(chain: usize) -> String {
    format!(
        r#"
name: synthetic-rules-v1
profile_version: "1.0.0"
node_kinds:
  item:
    id_pattern: '^N[0-9]+$'
    attrs:
      ordinal: {{ type: int, required: true }}
      status: {{ type: enum, required: true, values: [active, retired] }}
edge_kinds:
  next:
    allowed: [[item, item]]
  cites:
    allowed: [[item, item]]
validations:
  - EDGE_WHERE:
      edge_kind: next
      reject:
        target: {{ status: {{ eq: retired }} }}
  - EDGE_WHERE:
      edge_kind: cites
      expect:
        edge: {{ weight: {{ lt: 2 }} }}
  - EDGE_WHERE:
      edge_kind: cites
      when:
        source: {{ status: {{ eq: retired }} }}
      reject:
        target: {{ status: {{ eq: retired }} }}
  - EDGE_COUNT:
      edge_kind: next
      node_kind: item
      max: 1
  - EDGE_COUNT:
      edge_kind: cites
      node_kind: item
      direction: in
      min: 1
      where:
        status: {{ eq: retired }}
  - SUPPRESS:
      code: UNTRACED
  - SUPPRESS:
      code: EDGE_WHERE
      node_ids: [{}]
"#,
        rules_suppressed(chain).join(", ")
    )
}

/// Assert the rules register reached the rule paths in the numbers its cadences give,
/// so a profile that silently stopped matching cannot pass as a timed rules run.
fn rules_reached(label: &str, chain: usize, issues: &[latt_core::types::Issue]) {
    let count = |code: &str| issues.iter().filter(|issue| issue.code == code).count();
    let into_retired = (1..chain)
        .filter(|index| index.is_multiple_of(RULES_RETIRED_EVERY))
        .count();
    let heavy_cites = (0..chain.saturating_sub(2))
        .filter(|index| index.is_multiple_of(RULES_CITE_EVERY))
        .filter(|index| (index / RULES_CITE_EVERY) % 3 == 2)
        .count();
    let forks = (0..chain.saturating_sub(2))
        .filter(|index| index % RULES_FORK_EVERY == RULES_FORK_AT)
        .count();
    let retired = chain.div_ceil(RULES_RETIRED_EVERY);
    assert_eq!(
        count("EDGE_WHERE"),
        into_retired + heavy_cites,
        "{label}: EDGE_WHERE"
    );
    assert_eq!(count("EDGE_COUNT"), forks + retired, "{label}: EDGE_COUNT");
    assert_eq!(
        issues.iter().filter(|issue| issue.suppressed).count(),
        rules_suppressed(chain).len() + 1,
        "{label}: suppressed"
    );
    assert_eq!(count("SUPPRESS_UNUSED"), 0, "{label}: SUPPRESS_UNUSED");
}

fn fixtures(manifest: &Value) -> Vec<Fixture> {
    let missing_every = manifest["missing_ordinal_every"]
        .as_u64()
        .expect("missing-attribute cadence") as usize;
    scales(manifest)
        .into_iter()
        .map(|scale| {
            let chain = || {
                scale
                    .chain
                    .expect("a generated register declares its chain length")
            };
            let (document, profile_yaml, profile_name) = match scale.generator.as_str() {
                "synthetic-chain-v1" => (
                    chain_document(&scale, missing_every),
                    PROFILE.to_owned(),
                    "synthetic-chain-v1",
                ),
                "synthetic-defects-v1" => (
                    defects_document(chain(), missing_every),
                    PROFILE.to_owned(),
                    "synthetic-chain-v1",
                ),
                "synthetic-rules-v1" => (
                    rules_document(chain()),
                    rules_profile(chain()),
                    "synthetic-rules-v1",
                ),
                "synthetic-located-v1" => (
                    located_document(chain(), missing_every),
                    PROFILE.to_owned(),
                    "synthetic-chain-v1",
                ),
                other => panic!("scale {} names unknown generator {other}", scale.label),
            };
            let parsed = parse_document(&document).expect("synthetic document parses");
            let graph = ingest_document(parsed).expect("synthetic document ingests");
            assert_eq!(
                ingest_text(&document).expect("synthetic document ingests from text"),
                graph,
                "{}: ingest_text and the JSON tree build the same graph",
                scale.label
            );
            let profile = service::prepare_profile(
                serde_norway::from_str(&profile_yaml).expect("synthetic profile YAML parses"),
                profile_name,
            )
            .expect("synthetic profile prepares");
            let issues = validate(&graph, &profile, false);
            assert_eq!(
                graph.iter_nodes().count(),
                scale.nodes,
                "{}: stored nodes",
                scale.label
            );
            assert_eq!(
                graph.iter_edges().count(),
                scale.edges,
                "{}: stored edges",
                scale.label
            );
            if matches!(
                scale.generator.as_str(),
                "synthetic-chain-v1" | "synthetic-located-v1"
            ) {
                assert_eq!(
                    issues
                        .iter()
                        .filter(|issue| issue.code == "ATTR_REQUIRED")
                        .count(),
                    scale.nodes.div_ceil(missing_every)
                );
            }
            if scale.generator == "synthetic-rules-v1" {
                rules_reached(&scale.label, chain(), &issues);
            }
            Fixture {
                scale,
                document,
                graph,
                profile,
                issues,
            }
        })
        .collect()
}

/// FNV-1a 64 over the record encoding `fixture.json` pins: fields joined by a tab,
/// each record ended by a newline, so a digest can be re-derived by hand.
struct Digest(u64);

impl Digest {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn record(&mut self, fields: &[&str]) {
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                self.bytes(b"\t");
            }
            self.bytes(field.as_bytes());
        }
        self.bytes(b"\n");
    }

    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

fn attrs_json(attrs: &serde_json::Map<String, Value>) -> String {
    serde_json::to_string(attrs).expect("attrs serialize")
}

fn issue_record(digest: &mut Digest, issue: &latt_core::types::Issue) {
    // The pins hold the wire's file and line; `Display` writes them as `file:line`.
    let at = issue.origin.to_string();
    let (file, line) = at
        .rsplit_once(':')
        .expect("an origin displays as file:line");
    digest.record(&[
        &issue.code,
        &issue.message,
        file,
        line,
        issue.node_id.as_deref().unwrap_or(""),
    ]);
}

/// Compute every pinned result through the entry points the benchmarks time.
fn computed(fixture: &Fixture) -> Value {
    let mut codes = BTreeMap::<&str, u64>::new();
    let mut order = Digest::new();
    for issue in &fixture.issues {
        *codes.entry(issue.code.as_str()).or_default() += 1;
        issue_record(&mut order, issue);
    }

    let mut adapter_issues = Digest::new();
    for issue in fixture.graph.adapter_issues() {
        issue_record(&mut adapter_issues, issue);
    }

    let mut graph = Digest::new();
    for node in fixture.graph.iter_nodes() {
        graph.record(&[
            "node",
            &node.id,
            &node.kind,
            &attrs_json(&node.attrs),
            node.provenance.file(),
            &node.provenance.line().map_or(0, u32::from).to_string(),
        ]);
        // After its node, in attr order; a node with none adds no record, so the
        // scales pinned before attr locations existed keep their digests.
        for (attr, at) in node.attr_provenance() {
            graph.record(&[
                "attr",
                attr,
                at.file(),
                &at.line().map_or(0, u32::from).to_string(),
            ]);
        }
    }
    for edge in fixture.graph.iter_edges() {
        graph.record(&[
            "edge",
            &edge.src,
            &edge.tgt,
            &edge.kind,
            &attrs_json(&edge.attrs),
            edge.provenance.file(),
            &edge.provenance.line().map_or(0, u32::from).to_string(),
        ]);
    }

    let json_findings = output_result(&fixture.issues, Format::Json);
    let mut json_digest = Digest::new();
    json_digest.bytes(json_findings.as_bytes());

    let edge_kinds: [&str; 0] = [];
    let reach = |check_resolved| {
        query::reach(
            &fixture.graph,
            &fixture.profile,
            "N0",
            &edge_kinds,
            Direction::Forward,
            &[],
            check_resolved,
        )
        .expect("the origin is declared")
    };
    let open = reach(false);
    let mut open_digest = Digest::new();
    for node in &open.nodes {
        open_digest.record(&[&node.id, &node.kind]);
    }
    let resolved = reach(true);
    let mut resolved_digest = Digest::new();
    let mut tainted = 0_u64;
    for node in &resolved.nodes {
        let is_tainted = node.tainted == Some(true);
        tainted += u64::from(is_tainted);
        resolved_digest.record(&[&node.id, &node.kind, &is_tainted.to_string()]);
    }

    let counts = counts_digest(&query::counts(&fixture.graph, &fixture.profile, &[]));
    let context = InputContext::default();
    let service_counts = match service_counts(fixture, &context)
        .expect("service counts succeed")
        .payload
    {
        Report::Counts(report) => counts_digest(&report),
        _ => "not a counts report".to_owned(),
    };

    json!({
        "findings": { "counts": codes, "order_fnv1a64": order.hex() },
        "adapter_issues_fnv1a64": adapter_issues.hex(),
        "graph_fnv1a64": graph.hex(),
        "json_findings": { "bytes": json_findings.len(), "fnv1a64": json_digest.hex() },
        "reach": { "count": open.nodes.len(), "fnv1a64": open_digest.hex() },
        "reach_resolved": {
            "count": resolved.nodes.len(),
            "tainted": tainted,
            "fnv1a64": resolved_digest.hex()
        },
        "counts_fnv1a64": counts,
        "service_counts_fnv1a64": service_counts
    })
}

fn counts_digest(counts: &CountsReport) -> String {
    let mut digest = Digest::new();
    for (side, tallies) in [("nodes", &counts.nodes), ("edges", &counts.edges)] {
        for (kind, count) in tallies {
            digest.record(&[side, kind, &count.to_string()]);
        }
    }
    digest.hex()
}

// body: service/counts
/// Send the request `service/counts` times; the pin reads its answer from here too.
fn service_counts(
    fixture: &Fixture,
    context: &InputContext,
) -> Result<Response<Report>, ServiceError> {
    service::execute(
        View {
            graph: &fixture.graph,
            profile: &fixture.profile,
            context,
        },
        Request::Counts { filters: &[] },
    )
}
// body-end

/// Fail at bench time when a benchmark is not in the shared operation list, and
/// declare its throughput when the list says the work scales with the fixture.
/// Criterion cannot unset a group's throughput once declared, so an operation
/// declared without one must run before every scaled operation in its group.
fn declare(group: &mut BenchmarkGroup<'_, WallTime>, name: &str, function: &str, scale: &Scale) {
    let operation = OPERATIONS
        .iter()
        .find(|operation| operation.group == name && operation.function == function)
        .unwrap_or_else(|| panic!("{name}/{function} is not a declared operation"));
    if operation.throughput == Some(ThroughputBasis::Nodes) {
        group.throughput(Throughput::Elements(scale.nodes as u64));
    }
}

/// Register every operation. `// body:` … `// body-end` mark the source each operation's
/// `body` version covers; a change inside one needs that version bumped (`operations`).
fn benchmark(c: &mut Criterion, fixtures: &[Fixture]) {
    let mut document_group = c.benchmark_group("document");
    for fixture in fixtures {
        declare(&mut document_group, "document", "parse", &fixture.scale);
        // body: document/parse
        document_group.bench_with_input(
            BenchmarkId::new("parse", &fixture.scale.label),
            &fixture.document,
            |b, input| {
                // Parsing owns its allocation and result destruction in real use.
                b.iter(|| black_box(parse_document(black_box(input)).expect("document parses")));
            },
        );
        // body-end
        declare(&mut document_group, "document", "ingest", &fixture.scale);
        // body: document/ingest
        document_group.bench_with_input(
            BenchmarkId::new("ingest", &fixture.scale.label),
            &fixture.document,
            |b, input| {
                b.iter_batched(
                    || parse_document(input).expect("document parses for ingest setup"),
                    |parsed| black_box(ingest_document(parsed).expect("document ingests")),
                    BatchSize::LargeInput,
                );
            },
        );
        // body-end
        // The pair times text to graph whole, each including its graph's
        // destruction, so they differ only in how the text is read.
        declare(
            &mut document_group,
            "document",
            "ingest-text",
            &fixture.scale,
        );
        // body: document/ingest-text
        document_group.bench_with_input(
            BenchmarkId::new("ingest-text", &fixture.scale.label),
            &fixture.document,
            |b, input| {
                b.iter(|| black_box(ingest_text(black_box(input)).expect("document ingests")));
            },
        );
        // body-end
        declare(
            &mut document_group,
            "document",
            "parse-and-ingest",
            &fixture.scale,
        );
        // body: document/parse-and-ingest
        document_group.bench_with_input(
            BenchmarkId::new("parse-and-ingest", &fixture.scale.label),
            &fixture.document,
            |b, input| {
                b.iter(|| {
                    let parsed = parse_document(black_box(input)).expect("document parses");
                    black_box(ingest_document(parsed).expect("document ingests"))
                });
            },
        );
        // body-end
    }
    document_group.finish();

    let mut validation_group = c.benchmark_group("validation");
    for fixture in fixtures {
        declare(
            &mut validation_group,
            "validation",
            "validate",
            &fixture.scale,
        );
        // body: validation/validate
        validation_group.bench_with_input(
            BenchmarkId::new("validate", &fixture.scale.label),
            fixture,
            |b, fixture| {
                b.iter(|| black_box(validate(&fixture.graph, &fixture.profile, false)));
            },
        );
        // body-end
    }
    validation_group.finish();

    let edge_kinds: [&str; 0] = [];
    let mut query_group = c.benchmark_group("query");
    for fixture in fixtures {
        declare(&mut query_group, "query", "counts", &fixture.scale);
        // body: query/counts
        query_group.bench_with_input(
            BenchmarkId::new("counts", &fixture.scale.label),
            fixture,
            |b, fixture| {
                b.iter(|| black_box(query::counts(&fixture.graph, &fixture.profile, &[])));
            },
        );
        // body-end
        declare(&mut query_group, "query", "reach", &fixture.scale);
        // body: query/reach
        query_group.bench_with_input(
            BenchmarkId::new("reach", &fixture.scale.label),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    black_box(
                        query::reach(
                            &fixture.graph,
                            &fixture.profile,
                            "N0",
                            &edge_kinds,
                            Direction::Forward,
                            &[],
                            false,
                        )
                        .expect("chain is reachable"),
                    )
                });
            },
        );
        // body-end
    }
    query_group.finish();

    let context = InputContext::default();
    let mut service_group = c.benchmark_group("service");
    for fixture in fixtures {
        declare(&mut service_group, "service", "counts", &fixture.scale);
        // body: service/counts
        service_group.bench_with_input(
            BenchmarkId::new("counts", &fixture.scale.label),
            fixture,
            |b, fixture| {
                b.iter(|| {
                    black_box(service_counts(fixture, &context).expect("service counts succeed"))
                });
            },
        );
        // body-end
    }
    service_group.finish();

    let mut output_group = c.benchmark_group("output");
    for fixture in fixtures {
        declare(&mut output_group, "output", "json-findings", &fixture.scale);
        // body: output/json-findings
        output_group.bench_with_input(
            BenchmarkId::new("json-findings", &fixture.scale.label),
            &fixture.issues,
            |b, issues| {
                b.iter(|| black_box(output_result(black_box(issues), Format::Json)));
            },
        );
        // body-end
    }
    output_group.finish();
}

fn main() {
    let manifest = manifest();
    let policy = &manifest["policy"];
    let mut criterion = Criterion::default()
        .warm_up_time(Duration::from_secs(
            policy["warm_up_seconds"].as_u64().expect("warm-up time"),
        ))
        .measurement_time(Duration::from_secs(
            policy["measurement_seconds"]
                .as_u64()
                .expect("measurement time"),
        ))
        .sample_size(policy["sample_size"].as_u64().expect("sample size") as usize)
        .configure_from_args();
    let fixtures = fixtures(&manifest);
    let scratch = adapters::Scratch::new();
    let adapter_fixtures = adapters::fixtures(&manifest, &scratch);
    let pinned: Vec<Pinned<'_>> = fixtures
        .iter()
        .map(|fixture| Pinned {
            label: &fixture.scale.label,
            expected: fixture.scale.expected.as_ref(),
            actual: computed(fixture),
        })
        .chain(adapter_fixtures.iter().map(|fixture| Pinned {
            label: &fixture.scale.label,
            expected: fixture.scale.expected.as_ref(),
            actual: adapters::computed(fixture),
        }))
        .collect();
    pins::gate(&pinned, || {
        benchmark(&mut criterion, &fixtures);
        adapters::benchmark(&mut criterion, &adapter_fixtures);
    })
    .unwrap_or_else(|refusal| panic!("{refusal}"));
    criterion.final_summary();
}
