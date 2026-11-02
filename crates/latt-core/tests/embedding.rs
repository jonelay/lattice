//! Synthetic actor views: this is a mock host proof, not a real host's settlement.
use latt_core::document::ingest_document;
use latt_core::profile::load_profile_value;
use latt_core::query::{self, Direction};
use serde_json::Value;
use std::collections::BTreeSet;
const PROFILE: &str = include_str!("fixtures/embedding/profile.yaml");
const A: &str = include_str!("fixtures/embedding/actor-a.json");
const B: &str = include_str!("fixtures/embedding/actor-b.json");

// Requirement: graph-service/Host authority and scope isolation
#[test]
fn actor_fixture_baseline_has_independent_expected_structure() {
    let profile = load_profile_value(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    for (document, readings, assertions) in [(A, 1, vec!["A1", "A2"]), (B, 2, vec!["B1"])] {
        let graph = ingest_document(serde_json::from_str(document).unwrap()).unwrap();
        let counts = query::counts(&graph, &profile, &[]);
        assert_eq!(counts.nodes["claim"], 1);
        assert_eq!(counts.nodes["reading"], readings);
        assert_eq!(counts.nodes["assertion"], 2);
        assert_eq!(counts.edges["about"], 2);
        assert_eq!(counts.edges["uses"], 2);
        let reach =
            query::reach(&graph, &profile, "R1", &[], Direction::Reverse, &[], false).unwrap();
        assert_eq!(
            reach
                .nodes
                .iter()
                .map(|n| n.id.as_str())
                .collect::<Vec<_>>(),
            assertions
        );
    }
    let mut bounded: Value = serde_json::from_str(A).unwrap();
    bounded["nodes"]
        .as_array_mut()
        .unwrap()
        .retain(|n| n["id"] != "C1");
    bounded["edges"]
        .as_array_mut()
        .unwrap()
        .retain(|e| e["tgt"] != "C1");
    let graph = ingest_document(bounded).unwrap();
    assert_eq!(graph.iter_nodes().count(), 3);
    assert_eq!(graph.iter_edges().count(), 2);
    assert!(
        !latt_core::validate::validate(&graph, &profile, false)
            .iter()
            .any(|i| i.code == "VACANCY")
    );
}

use latt_core::output::{Format, output_result};
use latt_core::service::{
    self, ErrorKind, InputContext, InputStatus, Report, Request, Scope, View,
};
use serde_json::json;
use std::path::Path;

fn fixture_events() -> Vec<Value> {
    let mut records = vec![];
    let mut edges = vec![];
    for (actor, text) in [("a", A), ("b", B)] {
        let doc: Value = serde_json::from_str(text).unwrap();
        for node in doc["nodes"].as_array().unwrap() {
            let shared = node["id"] == "C1" || node["id"] == "R1";
            if actor == "b" && shared {
                continue;
            }
            let mut node = node.clone();
            node["attrs"]["private"] = json!("SECRET upstream attr");
            records.push(
                json!({"visible_to":if shared {vec!["a","b"]} else {vec![actor]},"value":node}),
            );
        }
        for edge in doc["edges"].as_array().unwrap() {
            edges.push(json!({"visible_to":[actor],"value":edge}));
        }
    }
    let finding = json!({"visible_to":["b"],"value":{"severity":"warning","code":"PARSE_ERROR","message":"R2 private source fragment","provenance":{"file":"private-B2.json","line":1}}});
    records
        .into_iter()
        .map(|record| json!({"collection":"nodes","record":record}))
        .chain(
            edges
                .into_iter()
                .map(|record| json!({"collection":"edges","record":record})),
        )
        .chain(std::iter::once(
            json!({"collection":"findings","record":finding}),
        ))
        .collect()
}

fn replay(events: &[Value]) -> Value {
    let mut world = json!({"nodes":[],"edges":[],"findings":[]});
    for event in events {
        world[event["collection"].as_str().unwrap()]
            .as_array_mut()
            .unwrap()
            .push(event["record"].clone());
    }
    world
}

// A deliberately small host adapter, outside production core. Authorize whole records
// and attrs before ingestion; changing context labels is not the authorization step.
fn project(world: &Value, actor: &str) -> Value {
    let visible = |record: &Value| {
        record["visible_to"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == actor)
    };
    let nodes: Vec<Value> = world["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| visible(r))
        .map(|r| {
            let mut node = r["value"].clone();
            node["attrs"]
                .as_object_mut()
                .unwrap()
                .retain(|k, _| k == "status" || k == "group");
            node
        })
        .collect();
    let ids: BTreeSet<_> = nodes.iter().map(|n| n["id"].as_str().unwrap()).collect();
    let edges: Vec<_> = world["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| visible(r))
        .map(|r| r["value"].clone())
        .filter(|e| {
            ids.contains(e["src"].as_str().unwrap()) && ids.contains(e["tgt"].as_str().unwrap())
        })
        .collect();
    let findings: Vec<_> = world["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| visible(r))
        .map(|r| r["value"].clone())
        .collect();
    json!({"interface_version":"1.2","nodes":nodes,"edges":edges,"findings":findings})
}

fn player_response(view: View<'_>, request: Request<'_>) -> String {
    match service::execute(view, request) {
        Ok(response) => format!(
            "{} {:?} {:?}",
            output_result(&response.payload, Format::Json),
            response.inputs,
            response.diagnostics
        ),
        Err(_) => "request unavailable in this view".into(),
    }
}

fn compare_for_player(
    actor: &str,
    before_actor: &str,
    before: View<'_>,
    after_actor: &str,
    after: View<'_>,
) -> Result<String, &'static str> {
    if actor != before_actor || actor != after_actor {
        return Err("comparison unavailable");
    }
    Ok(output_result(
        &service::compare("before", before, "after", after)
            .unwrap()
            .payload,
        Format::Json,
    ))
}

// Requirement: graph-service/Host authority and scope isolation
#[test]
fn authorized_projection_sweeps_every_request_and_comparison_without_disclosure_or_mutation() {
    let events = fixture_events();
    let initial_events = events.clone();
    let world = replay(&events);
    let initial_world = world.clone();
    let profile =
        service::prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    let kinds: [&str; 0] = [];
    for actor in ["a", "b"] {
        // Applied frontier, policy revision and projection version belong to this
        // host wrapper; only its opaque safe token is supplied to Lattice.
        let host_context = (actor, 42_u64, 3_u64, "fixture-v1");
        let context = InputContext::default()
            .with_identity(format!(
                "view-{}-{}-{}",
                host_context.0, host_context.1, host_context.2
            ))
            .with_scope(Scope::Declared {
                label: actor.into(),
                bounded: false,
            })
            .with_status(InputStatus::CompleteForDeclaredScope);
        let document = project(&world, actor);
        let original_document = document.clone();
        let graph = ingest_document(document.clone()).unwrap();
        let before_graph = graph.clone();
        let view = View {
            graph: &graph,
            profile: &profile,
            context: &context,
        };
        let origin = if actor == "a" { "A1" } else { "B1" };
        let requests = vec![
            Request::Reach {
                origin,
                direction: Direction::Forward,
                edge_kinds: &kinds,
                filters: &[],
                check_resolved: true,
            },
            Request::Reach {
                origin: "R1",
                direction: Direction::Reverse,
                edge_kinds: &kinds,
                filters: &[],
                check_resolved: false,
            },
            Request::Path {
                src: origin,
                tgt: "R1",
                edge_kinds: &kinds,
            },
            Request::Path {
                src: "R1",
                tgt: origin,
                edge_kinds: &kinds,
            },
            Request::Orphans {
                kind: None,
                filters: &[],
            },
            Request::Counts { filters: &[] },
            Request::Coverage,
            Request::At {
                target: Path::new(""),
                path: "crates/latt-core/tests/fixtures/embedding",
                host_path_exists: false,
                filters: &[],
            },
            Request::Validate { strict: false },
            Request::Validate { strict: true },
            Request::Summary,
            Request::Trace { strict: false },
            Request::Reach {
                origin: "RX",
                direction: Direction::Forward,
                edge_kinds: &kinds,
                filters: &[],
                check_resolved: false,
            },
            Request::Path {
                src: origin,
                tgt: "RX",
                edge_kinds: &kinds,
            },
            Request::Orphans {
                kind: Some("hidden-kind"),
                filters: &[],
            },
            Request::At {
                target: Path::new(""),
                path: "hidden-file",
                host_path_exists: false,
                filters: &[],
            },
        ];
        for request in requests {
            let response = player_response(view, request);
            for private in if actor == "a" {
                vec!["R2", "B1", "B2", "SECRET"]
            } else {
                vec!["A1", "A2", "SECRET"]
            } {
                assert!(
                    !response.contains(private),
                    "{actor}: {private} in {response}"
                );
            }
        }
        let counts = service::execute(view, Request::Counts { filters: &[] }).unwrap();
        let Report::Counts(ref report) = counts.payload else {
            panic!()
        };
        assert_eq!(report.nodes["reading"], if actor == "a" { 1 } else { 2 });
        assert_eq!(report.edges.values().sum::<i64>(), 4);
        if actor == "a" {
            let request = |origin| Request::Reach {
                origin,
                direction: Direction::Forward,
                edge_kinds: &kinds,
                filters: &[],
                check_resolved: false,
            };
            assert_eq!(
                player_response(view, request("R2")),
                player_response(view, request("RX"))
            );
        } else {
            assert!(
                service::execute(
                    view,
                    Request::Reach {
                        origin: "R2",
                        direction: Direction::Forward,
                        edge_kinds: &kinds,
                        filters: &[],
                        check_resolved: false,
                    },
                )
                .is_ok()
            );
        }
        let other_context = context.clone().with_identity("other-actor");
        assert!(
            compare_for_player(
                actor,
                actor,
                view,
                "other",
                View {
                    context: &other_context,
                    ..view
                }
            )
            .is_err()
        );
        assert!(compare_for_player(actor, actor, view, actor, view).is_ok());
        let replayed = ingest_document(project(&replay(&events), actor)).unwrap();
        let replay = service::execute(
            View {
                graph: &replayed,
                ..view
            },
            Request::Counts { filters: &[] },
        )
        .unwrap();
        assert_eq!(
            output_result(&counts.payload, Format::Json),
            output_result(&replay.payload, Format::Json)
        );
        assert_eq!(before_graph, graph);
        assert_eq!(document, original_document);
        drop(graph);
        drop(replayed);
        drop(context);
        assert!(output_result(&counts.payload, Format::Json).contains("reading"));
    }
    assert_eq!(world, initial_world);
    assert_eq!(events, initial_events);
}

// Requirement: graph-service/Separate input context diagnostics and execution
#[test]
fn actor_variations_keep_declared_input_status_and_owned_reports() {
    let profile =
        service::prepare_profile(serde_norway::from_str(PROFILE).unwrap(), "fixture").unwrap();
    for variant in ["bounded", "partial", "unknown"] {
        let mut document: Value = serde_json::from_str(A).unwrap();
        let mut context = InputContext::default();
        if variant == "bounded" {
            document["nodes"]
                .as_array_mut()
                .unwrap()
                .retain(|n| n["id"] != "C1");
            document["edges"]
                .as_array_mut()
                .unwrap()
                .retain(|e| e["tgt"] != "C1");
            context.scope = Scope::Declared {
                label: "neighborhood".into(),
                bounded: true,
            };
            context.input_status = InputStatus::CompleteForDeclaredScope;
        } else if variant == "partial" {
            context.scope = Scope::Declared {
                label: "selected".into(),
                bounded: false,
            };
            context.input_status = InputStatus::Partial;
            document["findings"] = json!([{"severity":"warning","code":"PARSE_ERROR","message":"visible fragment","provenance":{"file":"visible.json","line":1}}]);
        }
        let graph = ingest_document(document).unwrap();
        let view = View {
            graph: &graph,
            profile: &profile,
            context: &context,
        };
        let response = service::execute(view, Request::Counts { filters: &[] }).unwrap();
        let Report::Counts(counts) = response.payload else {
            panic!()
        };
        assert_eq!(
            counts.nodes.values().sum::<i64>(),
            if variant == "bounded" { 3 } else { 4 }
        );
        assert_eq!(
            counts.edges.values().sum::<i64>(),
            if variant == "bounded" { 2 } else { 4 }
        );
        assert_eq!(
            response.diagnostics.len(),
            usize::from(variant == "partial")
        );
        assert_eq!(response.inputs[0].context, context);
        if variant == "bounded" {
            let error = service::execute(
                view,
                Request::Reach {
                    origin: "C1",
                    direction: Direction::Forward,
                    edge_kinds: &[],
                    filters: &[],
                    check_resolved: false,
                },
            )
            .unwrap_err();
            assert_eq!(error.kind, ErrorKind::InvalidRequest);
            assert_eq!(error.inputs[0].context, context);
        }
        let validation = service::execute(view, Request::Validate { strict: false }).unwrap();
        let Report::Validate(report) = validation.payload else {
            panic!()
        };
        assert!(!report.findings.iter().any(|i| i.code == "VACANCY"));
    }
}
