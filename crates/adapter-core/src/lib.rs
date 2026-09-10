use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

pub const INTERFACE_VERSION: &str = "1.2";

#[derive(Debug, Serialize)]
pub struct Document<'a> {
    pub interface_version: &'static str,
    pub nodes: Vec<Node<'a>>,
    pub edges: Vec<Edge<'a>>,
    pub pathways: Vec<Pathway>,
    pub findings: Vec<Issue>,
}

impl Default for Document<'_> {
    fn default() -> Self {
        Self {
            interface_version: INTERFACE_VERSION,
            nodes: Vec::new(),
            edges: Vec::new(),
            pathways: Vec::new(),
            findings: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Node<'a> {
    pub id: String,
    pub kind: &'a str,
    pub attrs: BTreeMap<String, Value>,
    pub provenance: Provenance,
}

#[derive(Debug, Serialize)]
pub struct Edge<'a> {
    pub src: String,
    pub tgt: String,
    pub kind: &'a str,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, Value>,
    pub provenance: Provenance,
}

#[derive(Debug, Serialize)]
pub struct Pathway {
    pub name: String,
    pub order: Vec<String>,
    pub current: String,
}

#[derive(Debug, Serialize)]
pub struct Issue {
    pub severity: &'static str,
    pub code: &'static str,
    pub message: String,
    pub provenance: Provenance,
    pub node_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Provenance {
    pub file: String,
    pub line: u32,
}

impl Provenance {
    pub fn new(file: impl Into<String>, line: u32) -> Self {
        Self {
            file: file.into(),
            line,
        }
    }

    pub fn new_file(file: impl Into<String>) -> Self {
        Self::new(file, 0)
    }
}

impl Document<'_> {
    pub fn parse_error(&mut self, message: impl Into<String>, file: impl Into<String>, line: u32) {
        self.issue("error", "PARSE_ERROR", message, file, line, None);
    }

    pub fn node_parse_error(
        &mut self,
        message: impl Into<String>,
        file: impl Into<String>,
        line: u32,
        node_id: String,
    ) {
        self.issue("error", "PARSE_ERROR", message, file, line, Some(node_id));
    }

    pub fn pathway_invalid(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.issue("warning", "PATHWAY_INVALID", message, file, 0, None);
    }

    pub fn external_ref(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.issue("info", "EXTERNAL_REF", message, file, 0, None);
    }

    fn issue(
        &mut self,
        severity: &'static str,
        code: &'static str,
        message: impl Into<String>,
        file: impl Into<String>,
        line: u32,
        node_id: Option<String>,
    ) {
        self.findings.push(Issue {
            severity,
            code,
            message: message.into(),
            provenance: Provenance::new(file, line),
            node_id,
        });
    }
}
