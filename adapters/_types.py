"""The types an adapter shares with the core, defined on the adapter's side.

The core and most adapters are Rust; the openspec and latt-suggest adapters
stay Python. The interface document is what carries these between the two
sides, so the definitions live here and the core reads them back out of the
serialized document.
"""

from __future__ import annotations

from dataclasses import dataclass
from enum import Enum


@dataclass(frozen=True, slots=True)
class Provenance:
    """Where a node, edge or issue came from in the source text.

    ``line`` is one-based, or 0 when there is no exact line, and at most
    4294967295: the core refuses a document with a line outside that range.
    """

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
