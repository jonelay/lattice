//! The one list of benchmark operations, read by both the harness and the report
//! normalizer so a group added to one cannot be absent from the other.
//!
//! `body` versions an operation's timed body separately from its ID: a change to what
//! the timed closure does bumps it, so records whose bodies differ are not compared as
//! one series. `tests/test_benchmark_tools.py` holds each body's source to the hash
//! `benchmarks/component/bodies.json` lists for its version.

// The harness reads the declarations; only the normalizer publishes their names.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThroughputBasis {
    Nodes,
}

/// What the timed interval holds besides the routine itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// Work done before each timed call and excluded from it.
    pub setup: Setup,
    /// Whether destroying the routine's input falls inside the timed interval.
    pub input_drop: Destroyed,
    /// Whether destroying the routine's result falls inside the timed interval.
    pub output_drop: Destroyed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup {
    /// The routine reads the prepared fixture by reference.
    None,
    /// `parse_document` builds the input outside timing, in Criterion's
    /// `LargeInput` batches.
    ParseDocument,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destroyed {
    Timed,
    Untimed,
    /// The routine borrows its input, so nothing is destroyed per call.
    Borrowed,
}

impl Setup {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::ParseDocument => "parse-document",
        }
    }
}

impl Destroyed {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timed => "timed",
            Self::Untimed => "untimed",
            Self::Borrowed => "borrowed",
        }
    }
}

/// Borrow the fixture and time the result's destruction: every `b.iter` operation.
const BORROWED: Timing = Timing {
    setup: Setup::None,
    input_drop: Destroyed::Borrowed,
    output_drop: Destroyed::Timed,
};

/// `iter_batched` over a parsed document: `ingest_document` consumes the document
/// inside timing, and Criterion drops the graphs after the batch's timer stops.
const PARSED: Timing = Timing {
    setup: Setup::ParseDocument,
    input_drop: Destroyed::Timed,
    output_drop: Destroyed::Untimed,
};

/// The manifest scales an operation runs at: every entry of `scales`, or the entries
/// of `adapter_scales` whose `adapter` names this native adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScaleSet {
    Graph,
    Adapter(&'static str),
}

pub struct Operation {
    pub group: &'static str,
    pub function: &'static str,
    pub throughput: Option<ThroughputBasis>,
    pub body: &'static str,
    pub timing: Timing,
    pub scales: ScaleSet,
}

const fn scaled(
    group: &'static str,
    function: &'static str,
    body: &'static str,
    timing: Timing,
) -> Operation {
    Operation {
        group,
        function,
        throughput: Some(ThroughputBasis::Nodes),
        body,
        timing,
        scales: ScaleSet::Graph,
    }
}

/// A native adapter writing its document from a generated register, timed at that
/// adapter's scales; its throughput is the ingested document's node count.
const fn adapter(function: &'static str, native: &'static str, body: &'static str) -> Operation {
    Operation {
        group: "adapter",
        function,
        throughput: Some(ThroughputBasis::Nodes),
        body,
        timing: BORROWED,
        scales: ScaleSet::Adapter(native),
    }
}

pub const OPERATIONS: [Operation; 12] = [
    scaled("document", "parse", "1", BORROWED),
    scaled("document", "ingest", "1", PARSED),
    scaled("document", "ingest-text", "1", BORROWED),
    scaled("document", "parse-and-ingest", "1", BORROWED),
    scaled("validation", "validate", "1", BORROWED),
    scaled("query", "counts", "1", BORROWED),
    scaled("query", "reach", "1", BORROWED),
    scaled("service", "counts", "2", BORROWED),
    scaled("output", "json-findings", "1", BORROWED),
    adapter("toml", "toml", "1"),
    adapter("md", "md", "1"),
    adapter("prose-xref", "prose-xref", "1"),
];
