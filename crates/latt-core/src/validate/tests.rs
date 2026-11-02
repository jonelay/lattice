//! Tests for the default severities and the finding-code table in `docs/profiles.md`.

mod default_severity_tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::graph::{EdgeSpec, LatticeGraph};
    use crate::profile::load_profile;
    use crate::types::{Provenance, Severity};
    use crate::validate::{FindingCode, default_severity, validate};

    #[test]
    fn warning_finding_codes_have_explicit_defaults() {
        assert_eq!(
            default_severity(FindingCode::Unreferenced),
            Severity::Warning
        );
        assert_eq!(default_severity(FindingCode::Untraced), Severity::Warning);
        assert_eq!(default_severity(FindingCode::Coverage), Severity::Warning);
        assert_eq!(
            default_severity(FindingCode::PathwayUnresolved),
            Severity::Warning
        );
    }

    fn dangling_severity(cross_source: bool) -> Severity {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "lattice-cross-source-{}-{}.yaml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let flag = if cross_source {
            "    cross_source: true\n"
        } else {
            ""
        };
        std::fs::write(
            &path,
            format!(
                "name: t\nprofile_version: \"1.0.0\"\n\
                 node_kinds:\n  req:\n    id_pattern: \"^REQ-[0-9]+$\"\n\
                 edge_kinds:\n  derives:\n    allowed: [[req, req]]\n{flag}"
            ),
        )
        .expect("profile fixture writes");
        let profile = load_profile(&path).expect("profile fixture loads");

        let mut graph = LatticeGraph::new();
        graph
            .add_node(
                "REQ-1",
                "req",
                serde_json::Map::default(),
                Provenance::new("requirements.md", 1),
            )
            .expect("source node is unique");
        graph.add_edge(
            EdgeSpec {
                src: "REQ-1".into(),
                tgt: "REQ-2".into(),
                kind: "derives".into(),
                attrs: serde_json::Map::default(),
            },
            Provenance::new("requirements.md", 1),
        );

        validate(&graph, &profile, false)
            .into_iter()
            .find(|finding| finding.code == "VACANCY")
            .expect("dangling edge produces a finding")
            .severity
    }

    #[test]
    fn cross_source_dangling_target_is_a_hint() {
        assert_eq!(dangling_severity(true), Severity::Hint);
    }

    #[test]
    fn ordinary_dangling_target_remains_an_error() {
        assert_eq!(dangling_severity(false), Severity::Error);
    }

    /// Hint is the floor, so the only thing a `VACANCY` override could do to a
    /// `cross_source` finding is promote it, which the validation spec forbids.
    /// The override is aimed at the code's error default, not at a hint, so it
    /// is not a refused promotion either: no `CONFIG_ERROR`.
    #[test]
    fn vacancy_override_applies_to_ordinary_edges_and_cross_source_stays_hint() {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "lattice-cross-source-override-{}-{}.yaml",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(
            &path,
            "name: t\nprofile_version: \"1.0.0\"\n\
             node_kinds:\n  req:\n    id_pattern: \"^REQ-[0-9]+$\"\n\
             edge_kinds:\n  derives:\n    allowed: [[req, req]]\n    cross_source: true\n\
             \x20 contains:\n    allowed: [[req, req]]\n\
             validations:\n  - VACANCY:\n      severity: warning\n",
        )
        .expect("profile fixture writes");
        let profile = load_profile(&path).expect("profile fixture loads");

        let mut graph = LatticeGraph::new();
        graph
            .add_node(
                "REQ-1",
                "req",
                serde_json::Map::default(),
                Provenance::new("requirements.md", 1),
            )
            .expect("source node is unique");
        for (tgt, kind) in [("REQ-2", "derives"), ("REQ-3", "contains")] {
            graph.add_edge(
                EdgeSpec {
                    src: "REQ-1".into(),
                    tgt: tgt.into(),
                    kind: kind.into(),
                    attrs: serde_json::Map::default(),
                },
                Provenance::new("requirements.md", 1),
            );
        }

        let findings = validate(&graph, &profile, false);
        let vacancy = |kind: &str| {
            let needle = format!("(kind '{kind}')");
            findings
                .iter()
                .find(|f| f.code == "VACANCY" && f.message.contains(&needle))
                .expect("dangling edge produces a finding")
                .severity
        };
        assert_eq!(vacancy("contains"), Severity::Warning);
        assert_eq!(vacancy("derives"), Severity::Hint);
        assert!(
            findings.iter().all(|f| f.code != "CONFIG_ERROR"),
            "{findings:?}"
        );
    }
}

/// The finding-code table in `docs/profiles.md` is the consumer's view of
/// `default_severity`; a row that disagrees with the code teaches the wrong
/// exit code.
mod docs_finding_code_table {
    use crate::validate::{FindingCode, default_severity};

    /// Adapter-emitted codes; the core has no variant for them, so the table
    /// can only be checked for their presence.
    const ADAPTER_CODES: &[&str] = &[
        "PARSE_ERROR",
        "AMBIGUOUS_REF",
        "UNREAD_ENTRIES",
        "WRAPPED_REF",
        "EXTERNAL_REF",
        "SOURCE_MISSING",
        "PATHWAY_INVALID",
        "NO_MATCHING_FILES",
    ];

    fn every_code() -> Vec<FindingCode> {
        use FindingCode::*;
        let all = vec![
            AttrEnum,
            AttrListItems,
            AttrRequired,
            AttrType,
            Constraint,
            PathwayUnresolved,
            ConfigError,
            Coverage,
            CoverageDeep,
            CoverageUnknown,
            Vacancy,
            EdgeConstraint,
            EmptyKind,
            EdgeCount,
            EdgeWhere,
            IdFormat,
            Unreferenced,
            Untraced,
            SuggestedEdge,
            SuggestionUnresolved,
            SuggestionUnranked,
            SuppressUnused,
            OverrideUnused,
            UnknownKind,
            NoteFileMissing,
            NoteInvalid,
            NoteUnused,
            NoteExpired,
        ];
        // Exhaustive on purpose: a new variant fails to compile here until the
        // list above carries it.
        match all[0] {
            AttrEnum | AttrListItems | AttrRequired | AttrType | Constraint | PathwayUnresolved
            | ConfigError | Coverage | CoverageDeep | CoverageUnknown | Vacancy
            | EdgeConstraint | EmptyKind | EdgeCount | EdgeWhere | IdFormat | Unreferenced
            | Untraced | SuggestedEdge | SuggestionUnresolved | SuggestionUnranked
            | SuppressUnused | OverrideUnused | UnknownKind | NoteFileMissing | NoteInvalid
            | NoteUnused | NoteExpired => {}
        }
        all
    }

    /// `(code, fires-when, default)` per row of the "Finding codes" table.
    fn table_rows() -> Vec<(String, String, String)> {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/profiles.md"
        ))
        .unwrap();
        let section = text
            .split("### Finding codes")
            .nth(1)
            .expect("docs/profiles.md has a Finding codes section");
        section
            .lines()
            .take_while(|line| !line.starts_with("### "))
            .filter(|line| line.starts_with("| `"))
            .map(|line| {
                let cells: Vec<&str> = line.trim_matches('|').split('|').collect();
                assert_eq!(cells.len(), 3, "{line}");
                let code = cells[0].trim().trim_matches('`').to_owned();
                (code, cells[1].trim().to_owned(), cells[2].trim().to_owned())
            })
            .collect()
    }

    // Requirement: development-verification/Verify documentation and supported feature configurations
    #[test]
    fn every_core_code_has_a_row_with_its_shipped_default() {
        let rows = table_rows();
        for code in every_code() {
            let row = rows
                .iter()
                .find(|(name, _, _)| name == code.as_str())
                .unwrap_or_else(|| panic!("{}: no row in docs/profiles.md", code.as_str()));
            assert_eq!(
                row.2,
                default_severity(code).as_str(),
                "{}: docs/profiles.md default disagrees with default_severity",
                code.as_str()
            );
        }
    }

    #[test]
    fn adapter_codes_have_a_row_marked_as_such() {
        let rows = table_rows();
        for code in ADAPTER_CODES {
            let row = rows
                .iter()
                .find(|(name, _, _)| name == code)
                .unwrap_or_else(|| panic!("{code}: no row in docs/profiles.md"));
            assert!(
                row.1.contains("adapter-emitted"),
                "{code}: the row must say the code is adapter-emitted"
            );
        }
        for (code, fires, _) in &rows {
            let known = FindingCode::from_str(code).is_some()
                || ADAPTER_CODES.contains(&&**code)
                || crate::fuse::CODES.contains(&&**code);
            assert!(known, "{code}: a row for a code nothing emits");
            assert_eq!(
                ADAPTER_CODES.contains(&&**code),
                fires.contains("adapter-emitted"),
                "{code}: only adapter codes are marked adapter-emitted"
            );
        }
    }

    #[test]
    fn fuse_codes_have_a_row_marked_as_such() {
        let rows = table_rows();
        for code in crate::fuse::CODES {
            let row = rows
                .iter()
                .find(|(name, _, _)| name == code)
                .unwrap_or_else(|| panic!("{code}: no row in docs/profiles.md"));
            assert!(
                row.1.ends_with("(fuse)"),
                "{code}: the row must say the code is fuse's"
            );
            assert_eq!(
                row.2, "error",
                "{code}: every composition code defaults to error"
            );
        }
    }

    /// `contracts/codes.tsv` pins every code a shipped component emits; the
    /// table must hold exactly those, so a code added anywhere needs a row.
    #[test]
    fn the_table_holds_exactly_the_pinned_codes() {
        let pinned = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../contracts/codes.tsv"
        ))
        .unwrap();
        let mut pinned: Vec<String> = pinned
            .lines()
            .filter_map(|line| line.split('\t').next())
            .filter(|code| !code.is_empty())
            .map(str::to_owned)
            .collect();
        let mut documented: Vec<String> =
            table_rows().into_iter().map(|(code, _, _)| code).collect();
        pinned.sort();
        documented.sort();
        assert_eq!(
            documented, pinned,
            "docs/profiles.md finding codes vs contracts/codes.tsv"
        );
    }
}
