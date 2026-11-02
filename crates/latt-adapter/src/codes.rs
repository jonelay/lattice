//! The finding codes adapters share.
//!
//! An adapter emits a code by its constant here, so the codes registry in
//! `contracts/codes.tsv` is built from these lists rather than found by search.
//! A code an adapter alone emits is its crate's own `CODES` entry.

/// Part of the register could not be read: a malformed row, an unreadable file,
/// a repeated ID.
pub const PARSE_ERROR: &str = "PARSE_ERROR";
/// A register's declared pathway does not hold.
pub const PATHWAY_INVALID: &str = "PATHWAY_INVALID";
/// A reference points outside the register being read, so no edge is built.
pub const EXTERNAL_REF: &str = "EXTERNAL_REF";
/// A node whose kind declares `id_names` names a file or definition the target
/// lacks.
pub const SOURCE_MISSING: &str = "SOURCE_MISSING";
/// The adapter's file globs matched nothing in a readable target.
pub const NO_MATCHING_FILES: &str = "NO_MATCHING_FILES";
/// A reference with no file on its line found no heading, and the previous line
/// ends with a path (the prose-xref adapter).
pub const WRAPPED_REF: &str = "WRAPPED_REF";
/// A reference matches more than one target, so no edge is built (the
/// prose-xref adapter).
pub const AMBIGUOUS_REF: &str = "AMBIGUOUS_REF";
/// An entry of a mapped array has a shape the profile does not read (the TOML
/// adapter).
///
/// It is reported at its line as written. Under a plain mapping the attr is not
/// set, since its readable part would be a list the register never declared.
pub const UNREAD_ENTRIES: &str = "UNREAD_ENTRIES";

/// Every code a shipped Rust adapter emits. One list, so the core can tell a
/// severity override for a shipped code from one for a code nothing ships.
pub const ALL: &[&str] = &[
    PARSE_ERROR,
    PATHWAY_INVALID,
    EXTERNAL_REF,
    SOURCE_MISSING,
    NO_MATCHING_FILES,
    WRAPPED_REF,
    AMBIGUOUS_REF,
    UNREAD_ENTRIES,
];
