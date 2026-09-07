from pathlib import Path

from conftest import load_yaml_profile as load_profile


PROFILE_PATH = Path(__file__).parent.parent / "profiles" / "requirements-rm.yaml"


class TestRmProfile:
    def test_loads_without_error(self):
        profile = load_profile(PROFILE_PATH)
        assert profile.name == "requirements-rm"

    # Requirement: Node kinds
    def test_five_node_kinds(self):
        profile = load_profile(PROFILE_PATH)
        assert set(profile.node_kinds.keys()) == {"need", "req", "spec-goal", "test", "sc"}

    # Requirement: Edge kinds
    def test_three_edge_kinds(self):
        profile = load_profile(PROFILE_PATH)
        assert set(profile.edge_kinds.keys()) == {"derives", "fulfills", "verifies"}

    # Requirement: ID patterns
    def test_need_id_pattern(self):
        profile = load_profile(PROFILE_PATH)
        p = profile.node_kinds["need"].id_pattern
        assert p.fullmatch("BN-1")
        assert p.fullmatch("UN-10")
        assert not p.fullmatch("REQ-0701")

    def test_req_id_pattern(self):
        profile = load_profile(PROFILE_PATH)
        p = profile.node_kinds["req"].id_pattern
        assert p.fullmatch("REQ-0701")
        assert not p.fullmatch("REQ-1")

    def test_spec_goal_id_pattern(self):
        profile = load_profile(PROFILE_PATH)
        p = profile.node_kinds["spec-goal"].id_pattern
        assert p.fullmatch("07-1.1")
        assert p.fullmatch("01-3.2")
        assert not p.fullmatch("1.1")

    def test_test_id_pattern(self):
        profile = load_profile(PROFILE_PATH)
        p = profile.node_kinds["test"].id_pattern
        assert p.fullmatch("tests/test_fem.py::test_fem_linear")
        assert p.fullmatch("tests/test_geo.py::TestGeometry::test_inrunner")
        assert p.fullmatch("tests/sub/test_module.py::test_fem_linear")
        assert not p.fullmatch("test_fem_linear")
        assert not p.fullmatch("helper_func")

    # Requirement: Need node attrs
    def test_need_attrs(self):
        profile = load_profile(PROFILE_PATH)
        attrs = profile.node_kinds["need"].attrs
        assert attrs["text"].required is True
        assert attrs["tier"].type == "enum"
        assert attrs["traces_to"].type == "list"

    # Requirement: Req node attrs
    def test_req_attrs(self):
        profile = load_profile(PROFILE_PATH)
        attrs = profile.node_kinds["req"].attrs
        assert attrs["text"].required is True
        assert attrs["domain"].required is True
        assert attrs["rationale"].required is False

    # Requirement: Test node attrs
    def test_test_attrs_carry_the_optional_docstring(self):
        profile = load_profile(PROFILE_PATH)
        attrs = profile.node_kinds["test"].attrs
        assert attrs["file"].required is True
        assert attrs["function"].required is True
        assert attrs["docstring"].type == "string"
        assert attrs["docstring"].required is False

    # Requirement: Profile version
    def test_profile_version_is_1_11(self):
        assert load_profile(PROFILE_PATH).profile_version == "1.11.0"

    # Requirement: Test node attrs
    def test_test_kind_declares_its_ranked_text(self):
        """The sidecar named `docstring` itself until this profile declared it.
        Without the declaration the migration drops it from ranking in silence."""
        kind = load_profile(PROFILE_PATH).node_kinds["test"]

        assert kind.text_attrs == ("function", "docstring")

    # Requirement: Spec-goal node attrs
    def test_spec_goal_attrs(self):
        profile = load_profile(PROFILE_PATH)
        attrs = profile.node_kinds["spec-goal"].attrs
        assert attrs["title"].required is True
        assert attrs["status"].type == "enum"
        assert attrs["status"].values == ("done", "partial", "todo", "blocked")

    # Requirement: Edge kinds
    def test_verifies_allowed_pairs(self):
        profile = load_profile(PROFILE_PATH)
        v = profile.edge_kinds["verifies"]
        assert ("test", "req") in v.allowed
        assert ("need", "req") not in v.allowed

    def test_derives_allowed_pairs(self):
        profile = load_profile(PROFILE_PATH)
        d = profile.edge_kinds["derives"]
        assert ("need", "need") in d.allowed
        assert ("req", "need") in d.allowed
        assert ("req", "sc") in d.allowed

    # Requirement: Coverage validation
    def test_coverage_validation_config(self):
        profile = load_profile(PROFILE_PATH)
        assert "COVERAGE" in profile.validation_overrides
        assert profile.validation_overrides["COVERAGE"] == "warning"

    # Requirement: Summary configuration
    def test_summary_validation_config(self):
        profile = load_profile(PROFILE_PATH)
        assert "SUMMARY" in profile.validation_configs
        cfgs = profile.validation_configs["SUMMARY"]
        assert len(cfgs) == 1
        cfg = cfgs[0]
        assert cfg["node_kind"] == "spec-goal"
        assert cfg["status_attr"] == "status"
        assert cfg["group_by_attr"] == "file"


SPEC_PATH = (
    Path(__file__).parent.parent / "openspec" / "specs" / "rm-profile" / "spec.md"
)


class TestSpecPatternSync:
    # Requirement: Spec quotes shipped ID patterns
    def test_spec_quotes_every_shipped_id_pattern(self):
        """Guard: an id_pattern change that skips the spec is a red test.

        The rm-profile spec quotes each pattern verbatim in backticks; a
        profile pattern the spec does not quote means the spec scenarios
        were written against a pattern that no longer ships (the 33e85de
        failure class).
        """
        profile = load_profile(PROFILE_PATH)
        spec_text = SPEC_PATH.read_text()
        for kind, node_kind in profile.node_kinds.items():
            pattern = node_kind.id_pattern.pattern
            assert f"`{pattern}`" in spec_text, (
                f"id_pattern for kind '{kind}' ({pattern}) is not quoted in "
                f"{SPEC_PATH.name}; sync the spec's ID-patterns requirement"
            )
