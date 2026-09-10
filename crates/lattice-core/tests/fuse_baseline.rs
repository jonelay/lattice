//! The fuse report, held to committed synthetic baselines.
//!
//! Mirrors `trace_baseline.rs`: a hermetic fixture exercises composition,
//! cross-source edge resolution, and COVERAGE through the standard validator.
//! Set `LATTICE_REGEN_BASELINES=1` to regenerate all three renderings.

use std::path::PathBuf;

use lattice_core::fuse::{assemble, load_fuse_profile, load_manifest, parse_trace};
use lattice_core::output::{output_result, strip_ansi};

fn baselines() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[test]
fn fuse_reproduces_the_synthetic_baselines_in_all_three_formats() {
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
    let report = assemble(&manifest, &profile, traces, false).expect("assembly succeeds");

    for format in ["plain", "json", "rich"] {
        let rendered = output_result(&report, format).expect("format is known");
        let got = format!("{}\n", strip_ansi(&rendered));

        if std::env::var_os("LATTICE_REGEN_BASELINES").is_some() {
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
