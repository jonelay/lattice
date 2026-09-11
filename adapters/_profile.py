"""The thin reader over the core's resolved profile document.

The core loads and fully validates the user's profile, then hands adapters a
resolved JSON document (`resolved_schema`). This module only restructures that
document into the dataclasses adapters consume; shape validation is not
repeated here, because a second validator is a second implementation to drift.
"""
from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path

SUPPORTED_RESOLVED_SCHEMA = "1"

_PATHWAY_BINDING_KEYS = frozenset({"pathway", "position_attr"})


@dataclass(frozen=True, slots=True)
class AttrSchema:
    type: str
    required: bool = False
    values: tuple[str, ...] | None = None
    items: str | None = None


@dataclass(frozen=True, slots=True)
class NodeKind:
    name: str
    id_pattern: re.Pattern[str]
    attrs: dict[str, AttrSchema] = field(default_factory=dict)
    summary_attr: str | None = None
    # None and () are different answers: absent leaves a ranking consumer to
    # fall back to summary_attr, empty declares the kind offers no text.
    text_attrs: tuple[str, ...] | None = None
    # A literal line prefix, never a regex: the core validates this key and this
    # program applies it, and the two run different pattern engines.
    text_chunk_line_prefix: str | None = None


@dataclass(frozen=True, slots=True)
class EdgeKind:
    name: str
    allowed: tuple[tuple[str, str], ...] = ()


@dataclass(frozen=True, slots=True)
class PathwayBinding:
    """Ties a finding code's severity to a node attr's position on a pathway.

    Carries no demotion target: a demoted finding becomes `info`, always. The
    only useful target is the quietest severity, and a configurable one would
    admit both promotion and a `warning` that `--strict` promotes straight back.
    """

    pathway: str
    position_attr: str


@dataclass(frozen=True, slots=True)
class Profile:
    """The declared vocabulary of a register: node kinds, edge kinds, checks.

    Sole authority on ID syntax and on which kind pairs an edge admits; adapters
    consult it rather than carrying rules of their own.
    """

    name: str
    profile_version: str
    node_kinds: dict[str, NodeKind]
    edge_kinds: dict[str, EdgeKind]
    validation_overrides: dict[str, str] = field(default_factory=dict)
    validation_configs: dict[str, list[dict]] = field(default_factory=dict)
    pathways: tuple[str, ...] = ()
    pathway_bindings: dict[str, PathwayBinding] = field(default_factory=dict)
    extra: dict = field(default_factory=dict)


class ProfileError(Exception):
    pass


def _parse_attr(raw) -> AttrSchema:
    """Build one attribute schema, accepting the bare-type-name shorthand."""
    if isinstance(raw, str):
        raw = {"type": raw}
    values = raw.get("values")
    return AttrSchema(
        type=raw["type"],
        required=raw.get("required", False),
        values=tuple(values) if values is not None else None,
        items=raw.get("items"),
    )


def _parse_node_kind(name: str, raw) -> NodeKind:
    raw = raw or {}
    return NodeKind(
        name=name,
        id_pattern=re.compile(raw["id_pattern"]),
        attrs={a: _parse_attr(v) for a, v in (raw.get("attrs") or {}).items()},
        summary_attr=raw.get("summary_attr"),
        # `.get` with no default, then a None check: an absent key and a
        # declared `[]` must not collapse to the same value here.
        text_attrs=(
            tuple(raw["text_attrs"]) if raw.get("text_attrs") is not None else None
        ),
        text_chunk_line_prefix=raw.get("text_chunk_line_prefix"),
    )


def _parse_edge_kind(name: str, raw) -> EdgeKind:
    raw = raw or {}
    return EdgeKind(
        name=name,
        allowed=tuple((src, tgt) for src, tgt in raw.get("allowed") or []),
    )


def load_profile(path: str | Path) -> Profile:
    """Read a core-resolved profile document.

    Raises ProfileError when the file is not a resolved document this reader
    supports. This is the one check that stays here, because it is what tells
    core output from a stray file.
    """
    path = Path(path)
    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except Exception as e:
        raise ProfileError(f"failed to read profile {path}: {e}") from e

    schema = raw.get("resolved_schema") if isinstance(raw, dict) else None
    if schema != SUPPORTED_RESOLVED_SCHEMA:
        raise ProfileError(
            f"profile {path}: not a resolved profile document "
            f"(resolved_schema {schema!r}, supported: "
            f"'{SUPPORTED_RESOLVED_SCHEMA}'). The core resolves profiles; "
            f"adapters are not run against raw profile files"
        )

    validation_overrides: dict[str, str] = {}
    validation_configs: dict[str, list[dict]] = {}
    pathway_bindings: dict[str, PathwayBinding] = {}
    for entry in raw.get("validations") or []:
        for code, config in entry.items():
            severity = config.get("severity")
            if severity is not None:
                validation_overrides[code] = severity
            validation_configs.setdefault(code, []).append(dict(config))
            if _PATHWAY_BINDING_KEYS & set(config):
                pathway_bindings[code] = PathwayBinding(
                    pathway=config["pathway"], position_attr=config["position_attr"],
                )

    known_keys = {"resolved_schema", "name", "profile_version", "node_kinds",
                  "edge_kinds", "validations", "pathways"}

    return Profile(
        name=raw["name"],
        profile_version=raw["profile_version"],
        node_kinds={k: _parse_node_kind(k, v)
                    for k, v in raw["node_kinds"].items()},
        edge_kinds={k: _parse_edge_kind(k, v)
                    for k, v in raw["edge_kinds"].items()},
        validation_overrides=validation_overrides,
        validation_configs=validation_configs,
        pathways=tuple(raw.get("pathways") or ()),
        pathway_bindings=pathway_bindings,
        extra={k: v for k, v in raw.items() if k not in known_keys},
    )
