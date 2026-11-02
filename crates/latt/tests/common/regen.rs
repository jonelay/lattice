//! The switch that turns a byte-baseline check into a re-capture.
//!
//! Compiled into each baseline suite through `#[path]`, so every suite reads
//! the switch the same way.

/// True when `LATTICE_REGEN_BASELINES` asks to re-capture the committed baselines.
///
/// Only `1` asks. A baseline re-captured in CI would make the check pass
/// against whatever the build produced, so the switch is refused there.
///
/// # Panics
///
/// Panics when the variable holds anything but `1`, so that `0` or `false`
/// cannot silently rewrite the baselines, and when it is set alongside `CI`.
pub fn regen_baselines() -> bool {
    let Some(value) = std::env::var_os("LATTICE_REGEN_BASELINES") else {
        return false;
    };
    assert!(
        value == "1",
        "LATTICE_REGEN_BASELINES must be unset or 1, not {value:?}"
    );
    assert!(
        std::env::var_os("CI").is_none(),
        "LATTICE_REGEN_BASELINES is refused in CI: a re-captured baseline checks nothing"
    );
    true
}
