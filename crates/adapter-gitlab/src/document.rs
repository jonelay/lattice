use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

const CONTRACT_VERSION: &str = "1.1";

#[derive(Debug, Serialize)]
pub(crate) struct Document<'a> {
    contract_version: &'static str,
    pub(crate) nodes: Vec<Node<'a>>,
    pub(crate) edges: Vec<Edge<'a>>,
    axes: Vec<Axis>,
    pub(crate) issues: Vec<Issue>,
}

impl Default for Document<'_> {
    fn default() -> Self {
        Self {
            contract_version: CONTRACT_VERSION,
            nodes: Vec::new(),
            edges: Vec::new(),
            axes: Vec::new(),
            issues: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Node<'a> {
    pub(crate) id: String,
    pub(crate) kind: &'a str,
    pub(crate) attrs: BTreeMap<String, Value>,
    pub(crate) provenance: Provenance,
}

#[derive(Debug, Serialize)]
pub(crate) struct Edge<'a> {
    pub(crate) src: String,
    pub(crate) tgt: String,
    pub(crate) kind: &'a str,
    pub(crate) provenance: Provenance,
}

#[derive(Debug, Serialize)]
struct Axis {
    name: String,
    order: Vec<String>,
    current: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Issue {
    pub(crate) severity: &'static str,
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) provenance: Provenance,
    pub(crate) node_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Provenance {
    pub(crate) file: String,
    line: u32,
}

impl Provenance {
    pub(crate) fn new(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            line: 0,
        }
    }
}

impl Document<'_> {
    pub(crate) fn parse_error(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.issues.push(Issue {
            severity: "error",
            code: "PARSE_ERROR",
            message: message.into(),
            provenance: Provenance::new(file),
            node_id: None,
        });
    }

    pub(crate) fn external_ref(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.issues.push(Issue {
            severity: "info",
            code: "EXTERNAL_REF",
            message: message.into(),
            provenance: Provenance::new(file),
            node_id: None,
        });
    }
}
