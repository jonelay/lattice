//! Cross-check lattice's validators against an engine lattice did not write.
//!
//! A resolved [`latt_core::profile::Profile`] becomes a Trustfall schema
//! ([`schema`]), a [`latt_core::graph::LatticeGraph`] is served to Trustfall
//! through an adapter built only on `latt-core`'s public items ([`adapter`]),
//! and each validation-plan entry with a Trustfall spelling is translated into
//! queries ([`translate`]). The cross-check ([`crosscheck`]) runs both over the
//! same graph and compares the `(code, node_id)` sets.
//!
//! This crate is development-only: it is not published, not installed, and not
//! part of the supported surface (development-verification spec, validators are
//! cross-checked against an independent engine).

pub mod adapter;
pub mod crosscheck;
pub mod deliberate;
pub mod generate;
pub mod mangle;
pub mod schema;
pub mod translate;
