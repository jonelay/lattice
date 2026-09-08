use serde::Serialize;

const CONTRACT_VERSION: &str = "1.1";

#[derive(Debug, Serialize)]
pub(crate) struct Document {
    contract_version: &'static str,
    pub(crate) nodes: Vec<Node>,
    pub(crate) edges: Vec<Edge>,
    axes: Vec<Axis>,
    issues: Vec<Issue>,
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
pub(crate) struct Node {
    pub(crate) id: String,
    pub(crate) kind: &'static str,
    pub(crate) attrs: Attributes,
    pub(crate) provenance: Provenance,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Attributes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) assignee: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Edge {
    pub(crate) src: String,
    pub(crate) tgt: String,
    pub(crate) kind: &'static str,
    pub(crate) provenance: Provenance,
}

#[derive(Debug, Serialize)]
struct Axis {
    name: String,
    order: Vec<String>,
    current: String,
}

#[derive(Debug, Serialize)]
struct Issue {
    severity: &'static str,
    code: &'static str,
    message: String,
    provenance: Provenance,
    node_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Provenance {
    file: String,
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

impl Document {
    pub(crate) fn parse_error(&mut self, message: impl Into<String>, file: impl Into<String>) {
        self.issues.push(Issue {
            severity: "error",
            code: "PARSE_ERROR",
            message: message.into(),
            provenance: Provenance::new(file),
            node_id: None,
        });
    }
}
