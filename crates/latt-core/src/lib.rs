//! The lattice core, in Rust: ingest, the graph model, profile loading,
//! validation, the trace and summary payloads, and the three output formats.
//!
//! Domain vocabulary lives in profiles: nothing in this crate knows what a
//! `REQ` or an `M0` is.
//!
//! Embedders should prefer [`service`], the supported interface: it runs
//! queries and reports over a graph the host already holds, with input
//! attribution. The pipeline underneath runs in this order:
//!
//! 1. load the profile — [`profile::load_profile`] for a file,
//!    [`service::prepare_profile`] for a value already in memory,
//!    [`service::prepare_profile_str`] for its text;
//! 2. obtain the interface document — run the adapter with
//!    [`document::resolve_adapter`] and [`document::run_adapter_spec`], which
//!    also ingests it, or take text the host already has to
//!    [`document::parse_document`];
//! 3. ingest it into a graph with [`document::ingest_document`];
//! 4. validate with [`service::execute`] and [`service::Request::Validate`];
//! 5. render with [`output::output_result`].
//!
//! Here steps 3 to 5 run over a document held as text, with the profile built
//! on hidden lines:
//!
//! ```
//! use latt_core::document::{ingest_document, parse_document};
//! use latt_core::output::{Format, output_result};
//! use latt_core::service::{InputContext, Request, View, execute, prepare_profile};
//! # let profile = prepare_profile(serde_norway::from_str(r#"
//! # name: demo
//! # profile_version: "1.0.0"
//! # node_kinds:
//! #   req:
//! #     id_pattern: "^REQ-\\d+$"
//! # edge_kinds: {derives: {allowed: [[req, req]]}}
//! # "#)?, "demo")?;
//! let text = r#"{
//!   "interface_version": "1.2",
//!   "nodes": [{"id": "REQ-1", "kind": "req", "attrs": {},
//!              "provenance": {"file": "reqs.md", "line": 3}}],
//!   "edges": [], "pathways": [], "findings": []
//! }"#;
//!
//! let graph = ingest_document(parse_document(text)?)?;
//! let context = InputContext::default();
//! let view = View { graph: &graph, profile: &profile, context: &context };
//! let answer = execute(view, Request::Validate { strict: false })?;
//! let rendered = output_result(&answer.payload, Format::Plain);
//!
//! // The profile lets a `req` derive from another, and the lone node has no
//! // edges, so it is both unreferenced and untraced; the declared edge kind
//! // nothing used is reported too, so a run that built no edges says so.
//! assert_eq!(rendered, "INFO EMPTY_KIND <profile>:0 edge kind 'derives' has no edge\n\
//!     WARNING UNREFERENCED reqs.md:3 node 'REQ-1' has no incoming edges\n\
//!     WARNING UNTRACED reqs.md:3 node 'REQ-1' has no outgoing edges");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Features
//!
//! - `native-adapters` (default): the markdown, TOML and prose-xref adapters run in
//!   process, as `document::NativeAdapter::{Md, Toml, ProseXref}`. Without it those
//!   formats need their external adapter programs.
//!
//! # Supported items
//!
//! Every item this documentation shows is supported. Items hidden from it are
//! either unstable, such as the `fuse` module, or internal to the `latt`
//! binary, and carry no promise.
//!
//! [`serde_norway`] is re-exported because [`service::prepare_profile`] takes
//! its `Value`: a host names the version this crate was built with.

pub use serde_norway;

#[cfg(feature = "native-adapters")]
#[doc(hidden)]
pub mod adapters;

#[doc(hidden)]
pub mod algo;
#[doc(hidden)]
pub mod describe;
pub mod document;
#[doc(hidden)]
pub mod fuse;
pub mod graph;
mod notes;
pub mod output;
pub mod profile;
#[doc(hidden)]
pub mod query;
mod scope;
pub mod service;
pub mod suggest;
#[doc(hidden)]
pub mod summary;
#[doc(hidden)]
pub mod template;
#[doc(hidden)]
pub mod trace;
#[doc(hidden)]
pub mod types;
pub mod validate;
