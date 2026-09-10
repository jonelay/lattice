"""The types an adapter shares with the core, defined on the adapter's side.

The core is Rust from 0.3.0; the adapters stay Python. These carried the same
meaning on both sides of the old in-process boundary, and the interface document
is now what carries it, so the definitions live here and the core reads them
back out of the serialized document.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum


@dataclass(frozen=True, slots=True)
class Provenance:
    """Where a node, edge or issue came from in the source text."""

    file: str
    line: int


class Severity(Enum):
    """Issue severity. Only ERROR sets a non-zero exit code; HINT is advice
    that never affects it, and nothing promotes a finding out of it."""

    ERROR = "error"
    WARNING = "warning"
    INFO = "info"
    HINT = "hint"


@dataclass(frozen=True, slots=True)
class Issue:
    """One finding, from either an adapter's parse or a validation check.

    `code` is the stable machine-readable name (PARSE_ERROR, VACANCY);
    `message` is prose for a human and is not a contract.
    """

    severity: Severity
    code: str
    message: str
    provenance: Provenance
    node_id: str | None = None


class PathwayError(Exception):
    pass


@dataclass(frozen=True, slots=True)
class Pathway:
    """An ordering pathway read from the target: its positions and where it stands now.

    Ingested data, on the same footing as nodes and edges — the register declares
    it and the adapter reads it. Nothing here is computed or defaulted.
    """

    name: str
    order: list[str] = field(default_factory=list)
    current: str = ""

    def is_member(self, position: str) -> bool:
        """True when `position` is one of this pathway's declared positions."""
        return position in self.order

    def is_after(self, position: str) -> bool:
        """True when `position` sits strictly later on the pathway than `current`.

        False for a non-member, which callers must screen with `is_member`
        first — the two cases mean different things and share no answer.
        """
        if position not in self.order:
            return False
        return self.order.index(position) > self.order.index(self.current)
