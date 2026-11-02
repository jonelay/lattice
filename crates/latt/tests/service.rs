//! Supported service contract, distinct from the transport's exit policy.
use latt_core::document::ingest_document;
use latt_core::output::Format;
use latt_core::profile::load_profile_value;
use latt_core::service::*;
use serde_json::json;
use std::path::Path;
#[cfg(all(unix, feature = "native-adapters"))]
use std::path::PathBuf;
const PROFILE: &str = include_str!("../../latt-core/tests/fixtures/embedding/profile.yaml");
const DOCUMENT: &str = include_str!("../../latt-core/tests/fixtures/embedding/actor-a.json");

/// A program that runs native adapter `name` out of process, as `latt adapter <name>`
/// of the `latt` under test. A wrapper rather than the `adapters/<name>` shim: this
/// test spawns it through the library, so it cannot keep an installed `latt` on PATH
/// from answering in its place.
#[cfg(all(unix, feature = "native-adapters"))]
fn external_adapter(name: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("latt-service-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let script = format!(
        "#!/bin/sh\nexec '{}' adapter {name} \"$@\"\n",
        env!("CARGO_BIN_EXE_latt")
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

// Requirement: graph-service/Guard supported profile preparation
#[test]
fn preparation_rejects_unresolved_inheritance_by_name() {
    for value in ["absent-parent.yaml", "null", "''"] {
        let raw: serde_norway::Value =
            serde_norway::from_str(&format!("{PROFILE}extends: {value}\n")).unwrap();
        let loader = load_profile_value(raw.clone(), "legacy").unwrap_err();
        assert!(loader.to_string().contains("unknown keys [\"extends\"]"));
        let error = prepare_profile(raw, "fixture").unwrap_err();
        assert!(error.to_string().contains("resolve 'extends' before"));
    }
    assert!(prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").is_ok());
    assert!(prepare_profile(serde_norway::from_str("[]").unwrap(), "fixture").is_err());
    assert!(ingest_document(json!({"interface_version":"9.0"})).is_err());
}

// Requirement: graph-service/Separate input context diagnostics and execution
#[test]
fn adapter_diagnostics_are_not_promoted_or_suppressed_with_payload_findings() {
    let profile = prepare_profile(
        serde_norway::from_str(&format!(
            "{PROFILE}validations:\n  - SUPPRESS: {{code: PARSE_ERROR}}\n"
        ))
        .unwrap(),
        "fixture",
    )
    .unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(DOCUMENT).unwrap();
    doc["findings"] = json!([{"severity":"warning","code":"PARSE_ERROR","message":"visible malformed row","provenance":{"file":"input.json","line":1}}]);
    let graph = ingest_document(doc).unwrap();
    let context = InputContext::default();
    let view = View {
        graph: &graph,
        profile: &profile,
        context: &context,
    };
    let response = execute(view, Request::Validate { strict: true }).unwrap();
    assert_eq!(response.diagnostics.len(), 1);
    assert_eq!(response.diagnostics[0].issue.severity, Severity::Warning);
    assert!(!response.diagnostics[0].issue.suppressed);
    let Report::Validate(report) = response.payload else {
        panic!()
    };
    let issue = report
        .findings
        .iter()
        .find(|i| i.code == "PARSE_ERROR")
        .unwrap();
    assert_eq!(issue.severity, Severity::Error);
    assert!(issue.suppressed);
    assert_eq!(response.execution, ExecutionStatus::Complete);
    assert_eq!(
        response.inputs[0].context.input_status,
        InputStatus::Unknown
    );
}

// Requirement: graph-service/Supported borrowed graph execution
#[test]
fn at_uses_host_facts_and_unknown_nodes_keep_error_context() {
    let profile = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let graph = ingest_document(serde_json::from_str(DOCUMENT).unwrap()).unwrap();
    let context = InputContext::default().with_identity("safe-view");
    let view = || View {
        graph: &graph,
        profile: &profile,
        context: &context,
    };
    assert!(
        execute(
            view(),
            Request::At {
                target: Path::new(""),
                path: "input-never-created",
                host_path_exists: true,
                filters: &[]
            }
        )
        .is_ok()
    );
    let error = execute(
        view(),
        Request::At {
            target: Path::new(""),
            path: "input-never-created",
            host_path_exists: false,
            filters: &[],
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidRequest);
    assert_eq!(
        error.inputs[0].context.identity.as_deref(),
        Some("safe-view")
    );
    let error = execute(
        view(),
        Request::Reach {
            origin: "RX",
            direction: Direction::Forward,
            edge_kinds: &[],
            filters: &[],
            check_resolved: false,
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidRequest);
    assert_eq!(error.message, "unknown node 'RX'");
}

/// `expected` is what the pre-service call rendered: a report.
fn assert_legacy<'e, E>(view: View<'_>, request: Request<'_>, expected: &'e E)
where
    &'e E: Into<latt_core::output::Payload<'e>>,
{
    let response = execute(view, request).unwrap();
    for &format in Format::ALL {
        assert_eq!(
            latt_core::output::output_result(&response.payload, format),
            latt_core::output::output_result(expected, format)
        );
    }
    assert_eq!(
        response
            .diagnostics
            .iter()
            .map(|d| d.issue.clone())
            .collect::<Vec<_>>(),
        latt_core::validate::resolve_adapter_issues(view.graph, view.profile)
    );
}

// Requirement: graph-service/Supported borrowed graph execution
#[test]
fn all_operations_preserve_existing_semantics_with_duplicates_parallel_edges_and_vacancies() {
    use latt_core::{query, summary, trace, validate};
    let profile = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let mut document: serde_json::Value = serde_json::from_str(DOCUMENT).unwrap();
    let duplicate = document["nodes"][0].clone();
    document["nodes"].as_array_mut().unwrap().push(duplicate);
    let parallel = document["edges"][0].clone();
    document["edges"].as_array_mut().unwrap().push(parallel);
    let mut vacancy = document["edges"][1].clone();
    vacancy["tgt"] = json!("R99");
    document["edges"].as_array_mut().unwrap().push(vacancy);
    let graph = ingest_document(document).unwrap();
    assert_eq!(graph.iter_nodes().count(), 4);
    assert_eq!(graph.iter_edges().count(), 6);
    assert!(
        graph
            .adapter_issues()
            .iter()
            .any(|i| i.message.contains("duplicate node"))
    );
    let context = InputContext::default();
    let view = View {
        graph: &graph,
        profile: &profile,
        context: &context,
    };
    let kinds: [&str; 0] = [];
    let filters = vec![query::parse_filter("status=settled").unwrap()];
    assert!(query::parse_filter("status").is_err());
    assert!(query::parse_filter("status~=[").is_err());
    for direction in [Direction::Forward, Direction::Reverse] {
        assert_legacy(
            view,
            Request::Reach {
                origin: "A1",
                direction,
                edge_kinds: &kinds,
                filters: &filters,
                check_resolved: true,
            },
            &Report::Reach(
                query::reach(&graph, &profile, "A1", &kinds, direction, &filters, true).unwrap(),
            ),
        );
    }
    for (src, tgt) in [("A1", "R1"), ("R1", "A1")] {
        assert_legacy(
            view,
            Request::Path {
                src,
                tgt,
                edge_kinds: &kinds,
            },
            &Report::Path(query::path(&graph, &profile, src, tgt, &kinds).unwrap()),
        );
    }
    assert_legacy(
        view,
        Request::Orphans {
            kind: None,
            filters: &filters,
        },
        &Report::Orphans(query::orphans(&graph, &profile, None, &filters).unwrap()),
    );
    assert_legacy(
        view,
        Request::Counts { filters: &filters },
        &Report::Counts(query::counts(&graph, &profile, &filters)),
    );
    assert_legacy(
        view,
        Request::Coverage,
        &Report::Coverage(query::coverage(&graph, &profile)),
    );
    assert_legacy(
        view,
        Request::Summary,
        &Report::Summary(summary::build_summary(&profile, &graph).unwrap()),
    );
    for strict in [false, true] {
        let issues = validate::validate(&graph, &profile, strict);
        assert!(issues.iter().any(|i| i.code == "VACANCY"));
        let response = execute(view, Request::Validate { strict }).unwrap();
        // The text formats render the findings as the bare list did; JSON
        // carries them under `findings` with `scope` beside.
        for &format in Format::ALL {
            let rendered = latt_core::output::output_result(&response.payload, format);
            let bare = latt_core::output::output_result(&issues, format);
            if format == Format::Json {
                let rendered: serde_json::Value = serde_json::from_str(&rendered).unwrap();
                let bare: serde_json::Value = serde_json::from_str(&bare).unwrap();
                assert_eq!(rendered["findings"], bare["findings"]);
                assert!(rendered["scope"]["nodes"].is_object(), "{rendered}");
            } else {
                assert_eq!(rendered, bare);
            }
        }
        let Report::Validate(report) = response.payload else {
            panic!("validate answers with a validate report");
        };
        assert_eq!(report.findings, issues);
        assert_legacy(
            view,
            Request::Trace { strict },
            &Report::Trace(trace::build_trace_report(
                &profile,
                &graph,
                issues,
                env!("CARGO_PKG_VERSION"),
            )),
        );
    }
    let target = Path::new("");
    let path = "crates/latt-core/tests/fixtures/embedding";
    assert_legacy(
        view,
        Request::At {
            target,
            path,
            host_path_exists: false,
            filters: &filters,
        },
        &Report::At(
            query::at(
                &graph,
                &profile,
                validate::validate(&graph, &profile, false),
                target,
                path,
                env!("CARGO_PKG_VERSION"),
                &filters,
            )
            .unwrap(),
        ),
    );
    let response = compare("before", view, "after", view).unwrap();
    assert!(response.payload.is_empty());
    assert_eq!(response.inputs.len(), 2);
    assert_eq!(response.diagnostics.len(), 2);
    assert_eq!(response.diagnostics[0].side, InputSide::Before);
    assert_eq!(response.diagnostics[1].side, InputSide::After);
    assert_eq!(
        latt_core::output::output_result(&response.payload, Format::Json),
        latt_core::output::output_result(
            &query::diff("before", &graph, "after", &graph),
            Format::Json
        )
    );
    assert_eq!(
        execute(
            view,
            Request::Orphans {
                kind: Some("missing"),
                filters: &[]
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
    let unknown = ["missing"];
    assert_eq!(
        execute(
            view,
            Request::Path {
                src: "A1",
                tgt: "R1",
                edge_kinds: &unknown
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidRequest
    );
}

// Requirement: graph-service/Separate input context diagnostics and execution
#[test]
fn input_status_is_never_inferred_and_errors_preserve_both_contexts() {
    let profile = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let graph = ingest_document(json!({"interface_version":"1.2","nodes":[]})).unwrap();
    for (scope, status) in [
        (Scope::Unknown, InputStatus::Unknown),
        (
            Scope::Declared {
                label: "subset".into(),
                bounded: true,
            },
            InputStatus::CompleteForDeclaredScope,
        ),
        (
            Scope::Declared {
                label: "partial".into(),
                bounded: false,
            },
            InputStatus::Partial,
        ),
    ] {
        let context = InputContext::default()
            .with_scope(scope)
            .with_status(status);
        let response = execute(
            View {
                graph: &graph,
                profile: &profile,
                context: &context,
            },
            Request::Counts { filters: &[] },
        )
        .unwrap();
        assert_eq!(response.inputs[0].context, context);
        assert!(response.diagnostics.is_empty());
        let Report::Counts(counts) = response.payload else {
            panic!()
        };
        assert!(counts.nodes.values().all(|n| *n == 0));
    }
    let valid = InputContext::default();
    let invalid = InputContext::default().with_status(InputStatus::CompleteForDeclaredScope);
    let before = View {
        graph: &graph,
        profile: &profile,
        context: &valid,
    };
    let after = View {
        context: &invalid,
        ..before
    };
    let error = compare("before", before, "after", after).unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidRequest);
    assert_eq!(error.inputs.len(), 2);
    assert_eq!(error.inputs[1].side, InputSide::After);
    assert_eq!(
        execute(after, Request::Counts { filters: &[] })
            .unwrap_err()
            .kind,
        ErrorKind::InvalidRequest
    );
    let invalid_profile = prepare_profile(
        serde_norway::from_str(&format!(
            "{PROFILE}validations:\n  - SUMMARY: {{node_kind: assertion}}\n"
        ))
        .unwrap(),
        "fixture",
    )
    .unwrap();
    assert_eq!(
        execute(
            View {
                profile: &invalid_profile,
                ..before
            },
            Request::Summary
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidConfiguration
    );
}

// Requirement: graph-service/Supported borrowed graph execution
#[test]
fn at_can_select_finding_only_provenance_without_a_real_file() {
    let profile = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let graph=ingest_document(json!({"interface_version":"1.2","nodes":[],"findings":[{"severity":"warning","code":"PARSE_ERROR","message":"bad row","provenance":{"file":"unreadable/data.json","line":1}}]})).unwrap();
    let context = InputContext::default();
    for path in ["unreadable/data.json", "unreadable/"] {
        let response = execute(
            View {
                graph: &graph,
                profile: &profile,
                context: &context,
            },
            Request::At {
                target: Path::new(""),
                path,
                host_path_exists: false,
                filters: &[],
            },
        )
        .unwrap();
        assert!(
            latt_core::output::output_result(&response.payload, Format::Json).contains("bad row")
        );
    }
}

// Requirement: graph-service/Supported borrowed graph execution
#[test]
#[cfg(all(unix, feature = "native-adapters"))]
fn native_and_external_inputs_produce_equal_service_answers() {
    use latt_core::document::{resolve_adapter, run_adapter_spec};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    for (adapter, nodes, edges) in [("md", 3, 3), ("toml", 21, 10)] {
        let profile =
            latt_core::profile::load_profile(&root.join(format!("profiles/{adapter}.yaml")))
                .unwrap();
        let resolved = latt_core::profile::resolved_document(&profile).unwrap();
        let native = resolve_adapter(Some(std::ffi::OsStr::new(adapter)), &profile).unwrap();
        let external_path = external_adapter(adapter);
        let external = resolve_adapter(Some(external_path.as_os_str()), &profile).unwrap();
        let target = root.join(format!("tests/fixtures/mini-{adapter}"));
        let bound = profile.adapter_timeout();
        let native = run_adapter_spec(&native, &resolved, &target, bound).unwrap();
        let external = run_adapter_spec(&external, &resolved, &target, bound).unwrap();
        let context = InputContext::default();
        let mut reports = vec![];
        for graph in [&native, &external] {
            assert_eq!(graph.iter_nodes().count(), nodes);
            assert_eq!(graph.iter_edges().count(), edges);
            let response = execute(
                View {
                    graph,
                    profile: &profile,
                    context: &context,
                },
                Request::Trace { strict: false },
            )
            .unwrap();
            reports.push(latt_core::output::output_result(
                &response.payload,
                Format::Json,
            ));
        }
        assert_eq!(reports[0], reports[1]);
    }
}

// Requirement: graph-service/Supported borrowed graph execution
#[test]
fn at_does_not_probe_a_real_path_when_host_fact_is_false() {
    let profile = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let graph = ingest_document(json!({"interface_version":"1.2","nodes":[]})).unwrap();
    let context = InputContext::default();
    let actual = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    assert!(actual.exists());
    let view = View {
        graph: &graph,
        profile: &profile,
        context: &context,
    };
    let request = |exists| Request::At {
        target: Path::new(""),
        path: actual.to_str().unwrap(),
        host_path_exists: exists,
        filters: &[],
    };
    assert_eq!(
        execute(view, request(false)).unwrap_err().kind,
        ErrorKind::InvalidRequest
    );
    let response = execute(view, request(true)).unwrap();
    let Report::At(report) = response.payload else {
        panic!()
    };
    assert!(report.entries.is_empty());
    assert!(report.findings.is_empty());
}

// Requirement: graph-service/Separate input context diagnostics and execution
#[test]
fn comparison_and_error_diagnostics_use_each_inputs_profile() {
    let warning = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let error = prepare_profile(
        serde_norway::from_str(&format!(
            "{PROFILE}validations:\n  - PARSE_ERROR: {{severity: error}}\n"
        ))
        .unwrap(),
        "fixture",
    )
    .unwrap();
    let graph=ingest_document(json!({"interface_version":"1.2","nodes":[],"findings":[{"severity":"warning","code":"PARSE_ERROR","message":"visible input problem","provenance":{"file":"input.json","line":1}}]})).unwrap();
    let before_context = InputContext::default().with_identity("before-input");
    let after_context = InputContext::default().with_identity("after-input");
    let before = View {
        graph: &graph,
        profile: &warning,
        context: &before_context,
    };
    let after = View {
        graph: &graph,
        profile: &error,
        context: &after_context,
    };
    let response = compare("first", before, "second", after).unwrap();
    assert_eq!(response.diagnostics[0].side, InputSide::Before);
    assert_eq!(response.diagnostics[0].issue.severity, Severity::Warning);
    assert_eq!(response.diagnostics[1].side, InputSide::After);
    assert_eq!(response.diagnostics[1].issue.severity, Severity::Error);
    let failure = execute(
        after,
        Request::Path {
            src: "missing",
            tgt: "missing",
            edge_kinds: &[],
        },
    )
    .unwrap_err();
    assert_eq!(failure.inputs[0].context, after_context);
    assert_eq!(failure.diagnostics.len(), 1);
    assert_eq!(failure.diagnostics[0].issue.severity, Severity::Error);
}

// Requirement: graph-service/Host-supplied validation date

#[test]
fn note_expiry_follows_the_host_date() {
    let dir = std::env::temp_dir().join(format!("lattice_service_note_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("p.yaml"),
        "name: t\nprofile_version: \"1.0.0\"\nannotations: notes.yaml\nnode_kinds:\n  req:\n    id_pattern: \"^REQ-\\\\d+$\"\nedge_kinds:\n  derives:\n    allowed: [[req, req]]\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("notes.yaml"),
        "- finding: {code: VACANCY, node: REQ-1}\n  note: Lands later.\n  until: 2026-09-30\n",
    )
    .unwrap();
    let profile = latt_core::profile::load_profile(&dir.join("p.yaml")).unwrap();
    let graph = ingest_document(json!({
        "interface_version": "1.0",
        "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {}, "provenance": {"file": "R.md", "line": 1}}],
        "edges": [{"src": "REQ-1", "tgt": "REQ-9", "kind": "derives", "provenance": {"file": "R.md", "line": 1}}],
    }))
    .unwrap();
    let codes_on = |day: &str| {
        let context = InputContext::default().with_today(CivilDate::parse(day).unwrap());
        let view = View {
            graph: &graph,
            profile: &profile,
            context: &context,
        };
        let Report::Validate(report) = execute(view, Request::Validate { strict: false })
            .unwrap()
            .payload
        else {
            panic!("validate returns findings");
        };
        report
            .findings
            .into_iter()
            .map(|issue| issue.code)
            .collect::<Vec<_>>()
    };
    assert!(codes_on("2026-10-01").contains(&"NOTE_EXPIRED".to_string()));
    assert!(!codes_on("2026-09-30").contains(&"NOTE_EXPIRED".to_string()));
}

#[test]
fn note_annotations_are_refused_on_a_value_prepared_profile() {
    let raw = serde_norway::from_str(&format!("{PROFILE}\nannotations: notes.yaml\n")).unwrap();
    let error = prepare_profile(raw, "host").unwrap_err().to_string();
    assert!(error.contains("annotations"), "{error}");
}

// Requirement: graph-service/Supported service shapes admit additions
#[test]
fn a_host_builds_its_input_context_through_builders() {
    let today = CivilDate::new(2026, 10, 1).unwrap();
    let scope = Scope::Declared {
        label: "actor-a".into(),
        bounded: true,
    };
    let context = InputContext::default()
        .with_identity("snapshot-7")
        .with_scope(scope.clone())
        .with_status(InputStatus::CompleteForDeclaredScope)
        .with_today(today);
    assert_eq!(context.identity.as_deref(), Some("snapshot-7"));
    assert_eq!(context.scope, scope);
    assert_eq!(context.input_status, InputStatus::CompleteForDeclaredScope);
    assert_eq!(context.today, Some(today));
    assert_eq!(InputContext::default().identity, None);
}

// Requirement: graph-service/Supported service shapes admit additions
#[test]
fn edge_kind_filters_are_borrowed_slices_and_keep_set_semantics() {
    let profile = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let graph = ingest_document(serde_json::from_str(DOCUMENT).unwrap()).unwrap();
    let context = InputContext::default();
    let view = View {
        graph: &graph,
        profile: &profile,
        context: &context,
    };
    let reach = |edge_kinds: &[&str]| {
        let response = execute(
            view,
            Request::Reach {
                origin: "A1",
                direction: Direction::Forward,
                edge_kinds,
                filters: &[],
                check_resolved: false,
            },
        )
        .unwrap();
        let Report::Reach(report) = response.payload else {
            panic!("reach answers with a reach report");
        };
        report
    };
    let about = reach(&["about"]);
    let ids: Vec<_> = about.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, ["C1"]);
    // Order and repeats in the slice do not reach the report: it names the
    // kinds as the sorted set did.
    let both = reach(&["uses", "about", "uses"]);
    assert_eq!(both.edge_kinds, ["about", "uses"]);
    assert_eq!(both.nodes.len(), 2);

    let path = execute(
        view,
        Request::Path {
            src: "A1",
            tgt: "C1",
            edge_kinds: &["about"],
        },
    )
    .unwrap();
    assert!(matches!(path.payload, Report::Path(ref p) if p.found_path().is_some()));

    // With two unknown kinds the error names the smaller, whatever the order.
    for edge_kinds in [&["zz", "yy"][..], &["yy", "zz"][..]] {
        let error = execute(
            view,
            Request::Path {
                src: "A1",
                tgt: "C1",
                edge_kinds,
            },
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::InvalidRequest);
        assert!(
            error.message.starts_with("unknown edge kind 'yy'"),
            "{}",
            error.message
        );
    }
}

// Requirement: graph-service/Guard supported profile preparation
#[test]
fn a_profile_prepares_from_text_as_from_its_value() {
    let from_text = prepare_profile_str(PROFILE, "fixture").unwrap();
    let from_value = prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    assert_eq!(format!("{from_text:?}"), format!("{from_value:?}"));

    let error = prepare_profile_str("name: [unclosed", "host-profile").unwrap_err();
    assert!(
        matches!(&error, ProfileError::Yaml { path, .. } if path == Path::new("host-profile")),
        "{error:?}"
    );
    assert!(error.to_string().contains("host-profile"));

    let inherited = prepare_profile_str(&format!("{PROFILE}extends: parent.yaml\n"), "child");
    assert!(
        inherited
            .unwrap_err()
            .to_string()
            .contains("resolve 'extends' before")
    );
}
