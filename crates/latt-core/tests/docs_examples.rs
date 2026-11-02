//! Every YAML and JSON example in `docs/` and `README.md` is checked, each by
//! a named path.
//!
//! A reference whose examples do not parse teaches the wrong schema. Each
//! documented file with examples has a claim table below: one entry per
//! block, keyed by a marker only that block carries, naming how it is
//! checked. A complete profile loads as written; an extending profile loads
//! over the complete one; a fragment is laid over a base profile that
//! declares the kinds it names; an md or TOML adapter block runs through its
//! native adapter; an adapter block for a binary crate the core does not link
//! is checked for the keys the core reads and nothing more, and its entry
//! says so; the interface-document example ingests with the counts the text
//! states; a `validate --format json` example parses with the keys every
//! finding carries; a copy of a shipped profile equals the file. A block no entry claims, a block two entries claim, an entry no
//! block matches, and a file with examples but no table all fail naming the
//! file and line, so a new example cannot go unchecked.

use std::path::{Path, PathBuf};
use std::time::Duration;

use latt_core::document::{INTERFACE_VERSION, ingest_document};
use latt_core::fuse::{load_fuse_profile, load_manifest};
use latt_core::profile::{Profile, load_profile, load_profile_value, resolved_document};
use latt_core::suggest::render_suggestions;
use serde_norway::{Mapping, Value};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `README.md` and every page under `docs/`, so a new page is scanned without
/// being registered here.
fn documented_files() -> Vec<String> {
    let mut files = vec!["README.md".to_owned()];
    let mut pages: Vec<String> = std::fs::read_dir(repo_root().join("docs"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".md"))
        .map(|name| format!("docs/{name}"))
        .collect();
    pages.sort();
    files.extend(pages);
    files
}

struct Block {
    line: usize,
    info: String,
    body: String,
}

/// Every fenced block in `file`, whatever its info string, with the line its
/// fence opens on.
fn fenced_blocks(file: &str) -> Vec<Block> {
    let text = std::fs::read_to_string(repo_root().join(file)).unwrap();
    let mut blocks = Vec::new();
    let mut open: Option<(&'static str, Block)> = None;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_start();
        match &mut open {
            None => {
                if let Some(fence) = ["```", "~~~"].into_iter().find(|f| line.starts_with(f)) {
                    let info = line[fence.len()..].trim().to_owned();
                    let block = Block {
                        line: index + 1,
                        info,
                        body: String::new(),
                    };
                    open = Some((fence, block));
                }
            }
            Some((fence, _)) if line == *fence => blocks.push(open.take().unwrap().1),
            Some((_, block)) => {
                block.body.push_str(raw);
                block.body.push('\n');
            }
        }
    }
    assert!(open.is_none(), "{file}: unclosed fence");
    blocks
}

/// The blocks whose info string is exactly `lang`. An info string that starts
/// like `lang` but is not it (`yml`, `yaml title=x`) fails rather than being
/// skipped as prose.
fn blocks_of(file: &str, lang: &str) -> Vec<Block> {
    let initial = lang.chars().next().unwrap();
    fenced_blocks(file)
        .into_iter()
        .filter(|block| {
            assert!(
                block.info == lang || !block.info.starts_with(initial),
                "{file}:{}: fence info `{}` is not `{lang}`; spell it out",
                block.line,
                block.info
            );
            block.info == lang
        })
        .collect()
}

fn parse_yaml(where_: &str, body: &str) -> Value {
    serde_norway::from_str(body).unwrap_or_else(|e| panic!("{where_}: not YAML: {e}"))
}

/// A profile declaring every kind and attr the `docs/profiles.md` fragments name.
const BASE: &str = r#"
name: docs-base
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-"
    attrs:
      status: { type: string }
      stage: { type: string }
  need: { id_pattern: "^N-" }
  test: { id_pattern: "^T-" }
  requirement:
    id_pattern: "^R-"
    attrs:
      status: { type: string }
      rationale: { type: string }
      priority: { type: string }
  spec-goal:
    id_pattern: "^G-"
    attrs:
      status: { type: string }
      file: { type: string }
edge_kinds:
  verifies: { allowed: [[test, req]] }
  derives: { allowed: [[req, need], [req, req]] }
  picks: { allowed: [[req, req]] }
"#;

/// Lay `fragment` over the base: maps merge, `validations` lists append, a bare
/// list is validation entries, and bare `attrs` belong to a node kind.
fn overlay(where_: &str, fragment: Value) -> Value {
    let mut base: Value = serde_norway::from_str(BASE).unwrap();
    let root = base.as_mapping_mut().unwrap();
    let fragment = match fragment {
        Value::Sequence(entries) => {
            Mapping::from_iter([(Value::from("validations"), Value::Sequence(entries))])
        }
        Value::Mapping(map) if map.contains_key("attrs") => {
            assert_eq!(
                map.len(),
                1,
                "{where_}: a bare attrs fragment carries other keys, which this overlay would drop"
            );
            let kinds = root
                .get_mut("node_kinds")
                .unwrap()
                .as_mapping_mut()
                .unwrap();
            let req = kinds.get_mut("req").unwrap().as_mapping_mut().unwrap();
            req.insert("attrs".into(), map["attrs"].clone());
            return base;
        }
        Value::Mapping(map) => map,
        other => panic!("{where_}: unexpected fragment shape: {other:?}"),
    };
    for (key, value) in fragment {
        match (root.get_mut(&key), value) {
            (Some(Value::Mapping(existing)), Value::Mapping(added)) => existing.extend(added),
            (Some(Value::Sequence(existing)), Value::Sequence(added)) => existing.extend(added),
            (_, value) => {
                root.insert(key, value);
            }
        }
    }
    base
}

/// The kinds a profile around a `docs/adapters.md` block declares. A
/// single-table md profile must declare exactly one node kind, so each adapter
/// block gets its own.
const MD_SINGLE_KINDS: &str = "node_kinds:\n  requirement:\n    id_pattern: \"^REQ-\"\n    \
                               attrs: { summary: { type: string } }\n\
                               edge_kinds:\n  traces_to: { allowed: [[requirement, requirement]] }\n";
const MD_MULTI_KINDS: &str = "node_kinds:\n  requirement:\n    id_pattern: \"^REQ-\"\n    \
                              attrs: { summary: { type: string }, status: { type: string } }\n  \
                              stakeholder:\n    id_pattern: \"^S-\"\n    \
                              attrs: { name: { type: string }, role: { type: string } }\n\
                              edge_kinds:\n  traces_to: { allowed: [[requirement, requirement]] }\n";
const TOML_KINDS: &str = "node_kinds:\n  item:\n    id_pattern: \".\"\n    \
                          attrs: { id: { type: string }, stage: { type: string } }\n  \
                          register: { id_pattern: \".\" }\n\
                          edge_kinds:\n  belongs_to: { allowed: [[item, register]] }\n  \
                          references: { allowed: [[item, item]] }\n";
const TOML_HELIX_KINDS: &str = "node_kinds:\n  language: { id_pattern: \"^language/\" }\n  \
                                grammar: { id_pattern: \"^grammar/\" }\n\
                                edge_kinds:\n  uses_grammar: { allowed: [[language, grammar]] }\n";
const PROSE_XREF_KINDS: &str = "node_kinds:\n  document: { id_pattern: \".\" }\n  \
                                section: { id_pattern: \".\" }\n\
                                edge_kinds:\n  xref: { allowed: [[section, section]] }\n";
const OPENSPEC_KINDS: &str = "node_kinds:\n  requirement: { id_pattern: \".\" }\nedge_kinds: {}\n";

/// How one documented block is checked.
enum Check {
    /// A complete profile, loaded as written; the file's later `Extends`
    /// block names it as parent.
    Complete,
    /// Loaded over the file's `Complete` block through real files.
    Extends,
    /// A notes file, named by `BASE` with a `stage` pathway, validated over an
    /// empty graph carrying that pathway: no entry may be `NOTE_INVALID`.
    Notes,
    /// Laid over `BASE` and loaded.
    Fragment,
    /// An `adapter:` block run through the named native adapter over an empty
    /// target, which rejects a configuration it cannot read.
    Native {
        adapter: &'static str,
        kinds: &'static str,
    },
    /// An `adapter:` block the core alone can check: `native` and the timeout
    /// keys are validated and everything else passes through opaque, so the
    /// checker pins what the core does read and the rest is not checked.
    CoreKeys {
        kinds: &'static str,
        check: fn(&Profile),
    },
    /// A fuse manifest, loaded from a scratch file.
    Manifest,
    /// A fuse profile, loaded from a scratch file.
    FuseProfile,
    /// A GitLab CI job in the restricted shape `tests/ci_recipes.py` reads
    /// without a YAML parser: one job, `script` a sequence of strings, and
    /// `allow_failure.exit_codes` naming 1. The guard runs the job; this pins
    /// that a real parser reads the block as the guard does.
    CiJob,
    /// An interface document, ingested; the counts are the ones the prose
    /// beside it states.
    Document {
        nodes: usize,
        edges: usize,
        findings: usize,
    },
    /// A `validate --format json` output: an object whose `findings` is a
    /// non-empty array, each finding carrying exactly the keys every finding
    /// carries.
    ValidateOutput,
    /// A suggestion document, rendered over a graph declaring `declared`: every
    /// entry renders as `SUGGESTED_EDGE`, none as unresolved or unranked.
    Suggestions { declared: &'static [&'static str] },
    /// A copy of a shipped file, equal to it byte for byte, so the page cannot
    /// show a profile the install does not hold.
    Shipped(&'static str),
}

/// The claim table of one file: a marker only one block carries, and its check.
fn claims(file: &str) -> Option<Vec<(&'static str, Check)>> {
    Some(match file {
        "docs/profiles.md" => vec![
            ("name: md\n", Check::Shipped("profiles/md.yaml")),
            ("name: decisions\n", Check::Complete),
            ("node_kinds:\n  req:", Check::Fragment),
            ("node_kinds:\n  issue:", Check::Fragment),
            ("attrs:\n  text:", Check::Fragment),
            ("edge_kinds:\n  verifies:", Check::Fragment),
            ("validations:\n  - COVERAGE:", Check::Fragment),
            ("- COVERAGE_DEEP:", Check::Fragment),
            ("source_where:", Check::Fragment),
            ("- SUMMARY:", Check::Fragment),
            ("- CONSTRAINT:", Check::Fragment),
            ("- EDGE_WHERE:", Check::Fragment),
            ("- EDGE_COUNT:", Check::Fragment),
            ("- SUPPRESS:", Check::Fragment),
            ("see ADR-12", Check::Fragment),
            ("pathways:", Check::Fragment),
            ("extends:", Check::Extends),
            ("- finding: {code: VACANCY", Check::Notes),
        ],
        "docs/adapters.md" => vec![
            (
                "native: md",
                Check::CoreKeys {
                    kinds: MD_SINGLE_KINDS,
                    check: |profile| assert_eq!(profile.native_adapter(), Some("md")),
                },
            ),
            (
                "timeout_seconds: 600",
                Check::CoreKeys {
                    kinds: MD_SINGLE_KINDS,
                    check: |profile| {
                        assert_eq!(profile.adapter_timeout(), Duration::from_secs(600));
                    },
                },
            ),
            (
                "\n  table:",
                Check::Native {
                    adapter: "md",
                    kinds: MD_SINGLE_KINDS,
                },
            ),
            (
                "- heading:",
                Check::Native {
                    adapter: "md",
                    kinds: MD_MULTI_KINDS,
                },
            ),
            (
                "id_key: id",
                Check::Native {
                    adapter: "toml",
                    kinds: TOML_KINDS,
                },
            ),
            (
                "id_key: name",
                Check::Native {
                    adapter: "toml",
                    kinds: TOML_HELIX_KINDS,
                },
            ),
            (
                "content_dirs:",
                Check::Native {
                    adapter: "prose-xref",
                    kinds: PROSE_XREF_KINDS,
                },
            ),
            // The openspec adapter is Python; tests/test_adapter_openspec.py
            // runs it over a profile carrying these keys.
            (
                "spec_dir:",
                Check::CoreKeys {
                    kinds: OPENSPEC_KINDS,
                    check: |_| {},
                },
            ),
            ("- PARSE_ERROR:", Check::Fragment),
            // The sidecar's profile for a fused graph, with qualified kinds.
            ("\"markdown:requirement\":", Check::Complete),
            (
                "\"interface_version\"",
                Check::Document {
                    nodes: 2,
                    edges: 1,
                    findings: 1,
                },
            ),
            (
                "\"suggestion_version\"",
                Check::Suggestions {
                    declared: &["REQ-2", "SPEC-4"],
                },
            ),
        ],
        "docs/ci.md" => vec![("allow_failure:", Check::CiJob)],
        "docs/json.md" => vec![("\"findings\": [", Check::ValidateOutput)],
        "docs/fuse.md" => vec![
            ("manifest_version:", Check::Manifest),
            ("profile_version:", Check::FuseProfile),
        ],
        _ => return None,
    })
}

/// The one entry of `table` whose marker `body` carries.
fn claim_for<'t>(
    where_: &str,
    table: &'t [(&'static str, Check)],
    body: &str,
) -> &'t (&'static str, Check) {
    let matches: Vec<&(&str, Check)> = table.iter().filter(|(m, _)| body.contains(m)).collect();
    match matches.as_slice() {
        [one] => one,
        [] => panic!("{where_}: no test path claims this example"),
        many => panic!(
            "{where_}: claimed by more than one entry: {:?}",
            many.iter().map(|(m, _)| m).collect::<Vec<_>>()
        ),
    }
}

/// Where the file's checks keep what an earlier block produced.
struct FileRun {
    scratch: PathBuf,
    complete: Option<String>,
}

impl FileRun {
    fn new(file: &str) -> Self {
        let tag = file.replace('/', "-");
        let scratch =
            std::env::temp_dir().join(format!("lattice-docs-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        Self {
            scratch,
            complete: None,
        }
    }

    fn run(&mut self, where_: &str, check: &Check, body: &str) -> Result<(), String> {
        match check {
            Check::Complete => {
                self.complete = Some(body.to_owned());
                load_profile_value(parse_yaml(where_, body), where_)
                    .map(drop)
                    .map_err(|e| e.to_string())
            }
            Check::Extends => {
                let parent = self
                    .complete
                    .as_ref()
                    .expect("the parent example comes first");
                std::fs::write(self.scratch.join("decisions.yaml"), parent).unwrap();
                let child = self.scratch.join("child.yaml");
                std::fs::write(&child, body).unwrap();
                load_profile(&child).map(drop).map_err(|e| e.to_string())
            }
            Check::Notes => {
                let profile = self.scratch.join("noted.yaml");
                std::fs::write(
                    &profile,
                    format!("{BASE}\npathways: [stage]\nannotations: notes.yaml\n"),
                )
                .unwrap();
                std::fs::write(self.scratch.join("notes.yaml"), body).unwrap();
                let profile = load_profile(&profile).map_err(|e| e.to_string())?;
                let graph = ingest_document(serde_json::json!({
                    "interface_version": "1.0",
                    "pathways": [{"name": "stage", "order": ["M1", "M2"], "current": "M1"}],
                    "nodes": [],
                }))
                .map_err(|e| e.to_string())?;
                let invalid: Vec<String> = latt_core::validate::validate(&graph, &profile, false)
                    .into_iter()
                    .filter(|issue| issue.code == "NOTE_INVALID")
                    .map(|issue| issue.message)
                    .collect();
                if invalid.is_empty() {
                    Ok(())
                } else {
                    Err(invalid.join("; "))
                }
            }
            Check::Fragment => {
                let value = overlay(where_, parse_yaml(where_, body));
                load_profile_value(value, where_)
                    .map(drop)
                    .map_err(|e| e.to_string())
            }
            Check::Native { adapter, kinds } => {
                let (profile, resolved) = adapter_profile(where_, kinds, body)?;
                run_native(where_, adapter, &profile, &resolved, &self.scratch)
            }
            Check::CoreKeys { kinds, check } => {
                let (profile, _) = adapter_profile(where_, kinds, body)?;
                check(&profile);
                Ok(())
            }
            Check::Manifest => {
                let path = self.scratch.join("fuse.yaml");
                std::fs::write(&path, body).unwrap();
                load_manifest(&path).map(drop).map_err(|e| e.to_string())
            }
            Check::FuseProfile => {
                let path = self.scratch.join("fuse-profile.yaml");
                std::fs::write(&path, body).unwrap();
                load_fuse_profile(&path)
                    .map(drop)
                    .map_err(|e| e.to_string())
            }
            Check::CiJob => {
                let Value::Mapping(jobs) = parse_yaml(where_, body) else {
                    return Err("not a mapping of jobs".to_owned());
                };
                let [(_, Value::Mapping(job))] = jobs.iter().collect::<Vec<_>>()[..] else {
                    return Err(format!("{} jobs, not one", jobs.len()));
                };
                let Some(Value::Sequence(script)) = job.get("script") else {
                    return Err("`script` is not a sequence".to_owned());
                };
                if !script.iter().all(Value::is_string) {
                    return Err("a `script` entry is not a plain string".to_owned());
                }
                let codes = job
                    .get("allow_failure")
                    .and_then(|a| a.get("exit_codes"))
                    .and_then(Value::as_sequence)
                    .ok_or("no `allow_failure.exit_codes` sequence")?;
                if codes.iter().any(|c| c.as_u64() == Some(1)) {
                    Ok(())
                } else {
                    Err("`allow_failure.exit_codes` does not name 1".to_owned())
                }
            }
            Check::Document {
                nodes,
                edges,
                findings,
            } => {
                let value: serde_json::Value =
                    serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
                let graph = ingest_document(value).map_err(|e| e.to_string())?;
                assert_eq!(graph.iter_nodes().count(), *nodes, "{where_}: nodes");
                assert_eq!(graph.iter_edges().count(), *edges, "{where_}: edges");
                assert_eq!(
                    graph.adapter_issues().len(),
                    *findings,
                    "{where_}: findings"
                );
                Ok(())
            }
            Check::Suggestions { declared } => {
                let value: serde_json::Value =
                    serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
                let nodes: Vec<serde_json::Value> = declared
                    .iter()
                    .map(|id| serde_json::json!({"id": id, "kind": "item", "attrs": {}, "provenance": {"file": "r.md", "line": 1}}))
                    .collect();
                let graph = ingest_document(
                    serde_json::json!({"interface_version": INTERFACE_VERSION, "nodes": nodes}),
                )
                .map_err(|e| e.to_string())?;
                let issues = render_suggestions(&value, &graph).map_err(|e| e.to_string())?;
                let entries = value["suggestions"].as_array().map_or(0, Vec::len);
                assert_eq!(issues.len(), entries, "{where_}: rendered hints");
                for issue in &issues {
                    assert_eq!(issue.code, "SUGGESTED_EDGE", "{where_}: {}", issue.message);
                }
                Ok(())
            }
            Check::Shipped(path) => {
                let shipped = std::fs::read_to_string(repo_root().join(path)).unwrap();
                if *body == shipped {
                    Ok(())
                } else {
                    Err(format!("differs from {path}"))
                }
            }
            Check::ValidateOutput => {
                const KEYS: [&str; 6] = ["code", "file", "line", "message", "node_id", "severity"];
                let value: serde_json::Value =
                    serde_json::from_str(body).map_err(|e| format!("not JSON: {e}"))?;
                let findings = value
                    .get("findings")
                    .and_then(serde_json::Value::as_array)
                    .filter(|f| !f.is_empty())
                    .ok_or("no non-empty `findings` array")?;
                for finding in findings {
                    let object = finding.as_object().ok_or("a finding is not an object")?;
                    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
                    keys.sort_unstable();
                    if keys != KEYS {
                        return Err(format!("a finding carries keys {keys:?}, not {KEYS:?}"));
                    }
                }
                Ok(())
            }
        }
    }
}

impl Drop for FileRun {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

/// A profile of `kinds` with the block's top-level keys laid over it, loaded
/// and resolved as an adapter would receive it.
fn adapter_profile(where_: &str, kinds: &str, body: &str) -> Result<(Profile, String), String> {
    let head = format!("name: docs-adapters\nprofile_version: \"1.0.0\"\n{kinds}");
    let mut profile: Value = serde_norway::from_str(&head).unwrap();
    let Value::Mapping(fragment) = parse_yaml(where_, body) else {
        panic!("{where_}: expected a mapping");
    };
    profile.as_mapping_mut().unwrap().extend(fragment);
    let profile = load_profile_value(profile, where_).map_err(|e| e.to_string())?;
    let resolved = resolved_document(&profile).map_err(|e| e.to_string())?;
    Ok((profile, resolved))
}

/// Run the named native adapter over an empty target: enough for it to reject
/// a configuration it cannot read, without needing a fixture register.
#[cfg(feature = "native-adapters")]
fn run_native(
    where_: &str,
    adapter: &str,
    profile: &Profile,
    resolved: &str,
    empty: &Path,
) -> Result<(), String> {
    use latt_core::document::{AdapterSpec, resolve_adapter, run_adapter_spec};
    let spec = resolve_adapter(Some(adapter.as_ref()), profile).map_err(|e| e.to_string())?;
    assert!(matches!(spec, AdapterSpec::Native(_)), "{where_}");
    run_adapter_spec(&spec, resolved, empty, profile.adapter_timeout())
        .map(drop)
        .map_err(|e| e.to_string())
}

/// Without the feature the block still loads through the core; the adapter
/// itself is exercised by the default-feature run of this same test.
#[cfg(not(feature = "native-adapters"))]
fn run_native(_: &str, _: &str, _: &Profile, _: &str, _: &Path) -> Result<(), String> {
    Ok(())
}

// Requirement: development-verification/Verify documentation and supported feature configurations
#[test]
fn every_documented_example_is_claimed_and_checked() {
    for file in documented_files() {
        let mut blocks = blocks_of(&file, "yaml");
        blocks.extend(blocks_of(&file, "json"));
        blocks.sort_by_key(|block| block.line);
        let Some(table) = claims(&file) else {
            assert!(
                blocks.is_empty(),
                "{file}:{}: a yaml or json example in a file with no claim table",
                blocks[0].line
            );
            continue;
        };
        let mut run = FileRun::new(&file);
        let mut used = vec![0usize; table.len()];
        for block in &blocks {
            let where_ = format!("{file}:{}", block.line);
            let claim = claim_for(&where_, &table, &block.body);
            used[table.iter().position(|c| std::ptr::eq(c, claim)).unwrap()] += 1;
            if let Err(error) = run.run(&where_, &claim.1, &block.body) {
                // The loader's own errors already name the location.
                if error.starts_with(&where_) {
                    panic!("{error}");
                }
                panic!("{where_}: {error}");
            }
        }
        for ((marker, _), count) in table.iter().zip(&used) {
            assert_eq!(
                *count,
                1,
                "{file}: the entry `{}` claims {count} blocks, not one",
                marker.escape_debug()
            );
        }
    }
}
