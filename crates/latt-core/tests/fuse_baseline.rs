//! The fuse report, held to committed synthetic baselines.
//!
//! Mirrors `trace_baseline.rs`: a hermetic fixture exercises composition,
//! cross-source edge resolution, and COVERAGE through the standard validator.
//! Set `LATTICE_REGEN_BASELINES=1` to regenerate all three renderings.

use std::path::PathBuf;

#[path = "common/regen.rs"]
mod regen;

use latt_core::fuse::{assemble, load_fuse_profile, load_manifest, parse_trace};
use latt_core::output::{Format, output_result, strip_ansi};
use latt_core::types::FuseReport;

fn baselines() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn synthetic_report() -> FuseReport {
    let base = baselines();
    let manifest = load_manifest(&base.join("synthetic.fuse-manifest.yaml")).expect("manifest");
    let profile =
        load_fuse_profile(&base.join("synthetic.fuse-profile.yaml")).expect("fuse profile");
    let src = std::fs::read_to_string(base.join("synthetic.fuse-src.json")).expect("src trace");
    let dst = std::fs::read_to_string(base.join("synthetic.fuse-dst.json")).expect("dst trace");
    let traces = vec![
        parse_trace(&src, "src").expect("src parses"),
        parse_trace(&dst, "dst").expect("dst parses"),
    ];
    assemble(&manifest, &profile, traces, false).expect("assembly succeeds")
}

#[test]
fn fuse_reproduces_the_synthetic_baselines_in_all_three_formats() {
    let base = baselines();
    let report = synthetic_report();

    for &format in Format::ALL {
        let rendered = output_result(&report, format);
        let got = format!("{}\n", strip_ansi(&rendered));

        if regen::regen_baselines() {
            std::fs::write(base.join(format!("synthetic.fuse.{format}.txt")), &got)
                .expect("baseline is writable");
            continue;
        }
        let want = std::fs::read_to_string(base.join(format!("synthetic.fuse.{format}.txt")))
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

#[test]
fn fuse_text_prints_its_findings_before_the_composed_graph() {
    let report = synthetic_report();

    for format in [Format::Plain, Format::Rich] {
        let rendered = strip_ansi(&output_result(&report, format));
        let lines: Vec<&str> = rendered.lines().collect();
        let first_node = lines
            .iter()
            .position(|line| line.starts_with("src:T1 [src:test]"))
            .expect("the node list is printed");
        let last_finding = lines
            .iter()
            .rposition(|line| line.starts_with("ERROR ") || line.starts_with("INFO "))
            .expect("the findings are printed");
        assert!(
            lines[0].starts_with("Fuse synthetic-fuse"),
            "{format}: header first"
        );
        assert!(
            last_finding < first_node,
            "{format}: a finding at line {} follows the first node at line {}",
            last_finding + 1,
            first_node + 1
        );
    }
}
