//! The trace report, held to committed synthetic baselines.
//!
//! The gate uses a synthetic register because its contract is byte-level output
//! stability, independent of private or checkout-specific consumer data. The
//! by-hand real-data run documented in `CLAUDE.md` separately checks adapter
//! behavior. Set `LATTICE_REGEN_BASELINES=1` to regenerate all three renderings
//! through the same output path this test protects.

use std::path::PathBuf;

use lattice_core::document::{ingest_document, parse_document};
use lattice_core::output::{output_result, strip_ansi};
use lattice_core::profile::load_profile;
use lattice_core::trace::build_trace_report;
use lattice_core::validate::validate;

/// The version the baselines were captured at. Task 5.1 normalizes this header
/// across the whole matrix for the same reason: it is the one field that must
/// differ between two cores at different versions.
const BASELINE_VERSION: &str = "0.2.0";

fn baselines() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// Requirement: Trace JSON stability
// Requirement: Trace entry structure
// Requirement: Trace report rich format
// Requirement: Trace report JSON format

#[test]
fn trace_reproduces_the_synthetic_baselines_in_all_three_formats() {
    let profile = load_profile(&baselines().join("synthetic.profile.yaml")).expect("profile loads");
    let text = std::fs::read_to_string(baselines().join("synthetic.document.json"))
        .expect("the stored document is committed beside the baselines");
    let graph = ingest_document(parse_document(&text).unwrap()).expect("document ingests");

    let issues = validate(&graph, &profile, false);
    let report = build_trace_report(&profile, &graph, issues, BASELINE_VERSION);

    for format in ["plain", "json", "rich"] {
        let rendered = output_result(&report, format).expect("format is known");
        // click.echo strips ANSI off a non-tty and appends the newline, so the
        // captured file carries neither colour nor a bare render.
        let got = format!("{}\n", strip_ansi(&rendered));

        // A deliberate behaviour change re-captures through the very path this
        // test checks; the diff of the committed files is the review surface.
        if std::env::var_os("LATTICE_REGEN_BASELINES").is_some() {
            std::fs::write(
                baselines().join(format!("synthetic.trace.{format}.txt")),
                &got,
            )
            .expect("baseline is writable");
            continue;
        }
        let want =
            std::fs::read_to_string(baselines().join(format!("synthetic.trace.{format}.txt")))
                .expect("baseline is committed");

        assert_eq!(
            got.lines().count(),
            want.lines().count(),
            "{format}: line count differs"
        );
        if got != want {
            let at = got
                .lines()
                .zip(want.lines())
                .position(|(a, b)| a != b)
                .expect("differing text with equal lines has a differing line");
            panic!(
                "{format}: first difference at line {}\n  got:  {:?}\n  want: {:?}",
                at + 1,
                got.lines().nth(at).unwrap(),
                want.lines().nth(at).unwrap()
            );
        }
    }
}
