"""The thin reader over the core's resolved profile document.

Shape validation lives in the core's loader (covered by `profile_schema.rs`);
the reader trusts a document the core produced and checks only that it is one.
"""
from __future__ import annotations

import json
from pathlib import Path

import pytest

from adapters._profile import ProfileError, load_profile
from conftest import resolve_profile

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture
def resolved_path(tmp_path):
    return resolve_profile(FIXTURES / "test-profile.yaml", tmp_path)


def _write_doc(tmp_path, data) -> Path:
    p = tmp_path / "profile.json"
    p.write_text(json.dumps(data) if isinstance(data, dict) else data)
    return p


def _minimal(**overrides) -> dict:
    doc = {
        "resolved_schema": "1",
        "name": "t",
        "profile_version": "1.0.0",
        "node_kinds": {"req": {"id_pattern": "^R$"}},
        "edge_kinds": {},
    }
    doc.update(overrides)
    return doc


class TestResolvedReader:
    # Requirement: Resolved profile document
    def test_resolved_document_loads(self, resolved_path):
        profile = load_profile(resolved_path)
        assert profile.name == "test-rm"
        assert profile.profile_version == "1.0.0"

    def test_node_kind_pattern_compiled(self, resolved_path):
        profile = load_profile(resolved_path)
        assert profile.node_kinds["req"].id_pattern.match("REQ-0001")
        assert not profile.node_kinds["req"].id_pattern.match("REQ-1")

    def test_attr_schemas_surface(self, resolved_path):
        attrs = load_profile(resolved_path).node_kinds["req"].attrs
        assert attrs["text"].required
        assert attrs["status"].values == ("done", "partial", "todo", "blocked")
        assert attrs["tags"].type == "list" and attrs["tags"].items == "string"
        assert attrs["priority"].type == "int"

    def test_edge_kind_allowed(self, resolved_path):
        profile = load_profile(resolved_path)
        assert profile.edge_kinds["verifies"].allowed == (("test", "req"),)

    def test_extra_sections_preserved(self, tmp_path):
        doc = _minimal(adapter={"paths": {"requirements": "R.md"}})
        profile = load_profile(_write_doc(tmp_path, doc))
        assert profile.extra["adapter"]["paths"]["requirements"] == "R.md"

    def test_validations_surface(self, tmp_path):
        doc = _minimal(
            pathways=["phase"],
            validations=[
                {"COVERAGE": {"severity": "error", "target_kind": "req",
                              "edge_kind": "verifies",
                              "pathway": "phase", "position_attr": "phase"}},
            ],
        )
        profile = load_profile(_write_doc(tmp_path, doc))
        assert profile.validation_overrides["COVERAGE"] == "error"
        assert profile.validation_configs["COVERAGE"][0]["target_kind"] == "req"
        assert profile.pathways == ("phase",)
        assert profile.pathway_bindings["COVERAGE"].pathway == "phase"

    def test_missing_resolved_schema_rejected(self, tmp_path):
        doc = _minimal()
        del doc["resolved_schema"]
        with pytest.raises(ProfileError, match="resolved_schema"):
            load_profile(_write_doc(tmp_path, doc))

    def test_unsupported_resolved_schema_rejected(self, tmp_path):
        with pytest.raises(ProfileError, match="resolved_schema"):
            load_profile(_write_doc(tmp_path, _minimal(resolved_schema="2")))

    def test_unparseable_document_rejected(self, tmp_path):
        with pytest.raises(ProfileError, match="failed to read"):
            load_profile(_write_doc(tmp_path, "node_kinds: {not json"))

    # Requirement: Core invokes the adapter with profile and target paths
    def test_resolved_document_is_yaml_readable(self, resolved_path):
        # The staged migration rests on JSON being a YAML subset: an adapter
        # still reading the profile path with a YAML library must see the same
        # data. This is the contract the external bipolaris adapter relies on.
        yaml = pytest.importorskip("yaml")
        text = resolved_path.read_text()
        assert yaml.safe_load(text) == json.loads(text)
