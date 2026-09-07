use serde::Serialize;

pub const CONTRACT_VERSION: &str = "1.1";

#[derive(Debug, Serialize)]
pub struct Document {
    pub contract_version: &'static str,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub axes: Vec<Axis>,
    pub issues: Vec<Issue>,
}

impl Default for Document {
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
pub struct Node {
    pub id: String,
    pub kind: &'static str,
    pub attrs: Attributes,
    pub provenance: Provenance,
}

#[derive(Debug, Default, Serialize)]
pub struct Attributes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct Edge {
    pub src: String,
    pub tgt: String,
    pub kind: &'static str,
    pub provenance: Provenance,
}

#[derive(Debug, Serialize)]
pub struct Axis {
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

#[derive(Debug, Serialize)]
pub struct Provenance {
    pub file: String,
    pub line: u32,
}

impl Provenance {
    pub fn new(file: impl Into<String>) -> Self {
        Self {
            file: file.into(),
            line: 0,
        }
    }
}

impl Document {
    pub fn parse_error(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.issues.push(Issue {
            severity: "error",
            code: "PARSE_ERROR",
            message: message.into(),
            provenance: Provenance::new(file),
            node_id: None,
        });
    }
}
