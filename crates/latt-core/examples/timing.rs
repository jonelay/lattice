//! Historical task 3.4 measurement: where the Rust core's time went on a stored document.
//!
//! An example rather than a flag on the spike binary. The shipped CLI should not
//! grow a measurement surface, and the whole-process cost is measured from
//! outside anyway. Reports the best of N, matching how 2.8 measured the Python
//! side. This remains investigation provenance; `tools/benchmark.sh` is the
//! canonical repeatable component harness.
//!
//! Usage:  cargo run --release --example timing -- <profile> <document>

use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use latt_core::document::{ingest_document, parse_document};
use latt_core::output::{Format, output_result};
use latt_core::profile::load_profile;
use latt_core::validate::validate;

/// Results go through `black_box` so the optimiser cannot delete the work being
/// measured. A discarded `validate` is a call the compiler is free to elide.
const RUNS: u32 = 5;

fn best(label: &str, runs: u32, mut body: impl FnMut()) {
    let mut best = f64::MAX;
    for _ in 0..runs {
        let start = Instant::now();
        body();
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!("{label:<20} {best:>8.2} ms");
}

fn best_with_setup<T>(
    label: &str,
    runs: u32,
    mut setup: impl FnMut() -> T,
    mut body: impl FnMut(T),
) {
    let mut best = f64::MAX;
    for _ in 0..runs {
        let input = setup();
        let start = Instant::now();
        body(input);
        best = best.min(start.elapsed().as_secs_f64() * 1000.0);
    }
    println!("{label:<20} {best:>8.2} ms");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let profile_path = args.next().expect("usage: timing <profile> <document>");
    let document_path = args.next().expect("usage: timing <profile> <document>");

    let text = std::fs::read_to_string(&document_path).expect("document is readable");

    best("load_profile", RUNS, || {
        black_box(load_profile(Path::new(&profile_path)).expect("profile loads"));
    });
    best("parse_document", RUNS, || {
        black_box(parse_document(&text).expect("document parses"));
    });

    best_with_setup(
        "ingest_document",
        RUNS,
        || parse_document(&text).expect("document parses"),
        |parsed| {
            black_box(ingest_document(parsed).expect("document ingests"));
        },
    );

    let profile = load_profile(Path::new(&profile_path)).expect("profile loads");
    let graph =
        ingest_document(parse_document(&text).expect("document parses")).expect("document ingests");
    best("validate", RUNS, || {
        black_box(validate(&graph, &profile, false));
    });

    let issues = validate(&graph, &profile, false);
    for &format in Format::ALL {
        best(&format!("output {format}"), RUNS, || {
            black_box(output_result(&issues, format));
        });
    }

    best("everything", RUNS, || {
        let profile = load_profile(Path::new(&profile_path)).expect("profile loads");
        let parsed = parse_document(&text).expect("document parses");
        let graph = ingest_document(parsed).expect("document ingests");
        let issues = validate(&graph, &profile, false);
        black_box(output_result(&issues, Format::Plain));
    });
}
