//! The built-in adapters: markdown tables, TOML registers and `§` prose
//! cross-references, each building a [`latt_adapter::Document`] like any other
//! Rust adapter.
//!
//! The native dispatch in [`crate::document`] is the only caller in this crate, so a
//! built-in adapter reaches the graph through ingest alone, exactly as an external
//! document does (`tests/adapters_boundary.rs`). The module is public for hosts that
//! run one directly, but hidden: an unstable item with no promise.

pub mod md;
pub mod prose_xref;
pub mod toml;

/// Return why a file of these bytes is not read, when it has more lines than a
/// provenance line can name.
///
/// Every built-in adapter checks before reading a file, so inside a read file
/// each counted line fits `u32` and no item is placed at a wrapped or clamped
/// line. The refused file is the unread item, reported at line 0.
pub(crate) fn too_many_lines(bytes: &[u8]) -> Option<String> {
    beyond_lines(bytes.iter().filter(|byte| **byte == b'\n').count() + 1)
}

/// The refusal for a file of `line_count` lines, if it has more than fit `u32`.
fn beyond_lines(line_count: usize) -> Option<String> {
    (line_count > u32::MAX as usize).then(|| {
        format!(
            "file has {line_count} lines, more than the {} a provenance line can name, \
             so it was not read",
            u32::MAX
        )
    })
}

#[cfg(test)]
mod tests {
    use super::beyond_lines;

    #[test]
    fn line_fits_reads_a_file_of_the_largest_line_count() {
        assert_eq!(beyond_lines(4_294_967_295), None);
    }

    #[test]
    fn line_fits_refuses_a_file_with_one_line_more() {
        let refusal = beyond_lines(4_294_967_296).expect("refused");
        assert!(refusal.contains("4294967296"), "{refusal}");
    }
}
