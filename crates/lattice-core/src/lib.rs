//! The lattice core, in Rust: ingest, the graph model, profile loading,
//! validation, the trace and summary payloads, and the three output formats.
//!
//! Domain vocabulary lives in profiles, never here. Nothing in this crate knows
//! what a `REQ` or an `M0` is.
//!
//! The pipeline is four steps, and a caller runs them in this order:
//!
//! ```
//! use lattice_core::document::{ingest_document, parse_document};
//! use lattice_core::output::output_result;
//! use lattice_core::validate::validate;
//! # use lattice_core::profile::load_profile;
//! # let profile_yaml = r#"
//! # name: demo
//! # profile_version: "1.0.0"
//! # node_kinds:
//! #   req:
//! #     id_pattern: "^REQ-\\d+$"
//! # edge_kinds: {}
//! # "#;
//! # let dir = std::env::temp_dir().join("lattice_core_doctest");
//! # std::fs::create_dir_all(&dir).unwrap();
//! # let profile_path = dir.join("demo.yaml");
//! # std::fs::write(&profile_path, profile_yaml).unwrap();
//! # let profile = load_profile(&profile_path).unwrap();
//! let text = r#"{
//!   "contract_version": "1.0",
//!   "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
//!              "provenance": {"file": "reqs.md", "line": 3}}],
//!   "edges": [], "axes": [], "issues": []
//! }"#;
//!
//! let graph = ingest_document(parse_document(text)?)?;
//! let issues = validate(&graph, &profile, false);
//! let rendered = output_result(&issues, "plain").unwrap();
//!
//! // The lone node has no edges, so it is an orphan.
//! assert_eq!(rendered, "WARNING ORPHAN_NODE reqs.md:3 node 'REQ-1' has no edges");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod document;
pub mod graph;
pub mod output;
pub mod profile;
pub mod query;
pub mod suggest;
pub mod summary;
pub mod trace;
pub mod types;
pub mod validate;
