"""Guard the documentation rules stated in CLAUDE.md.

Comment density here has historically tracked edit recency rather than what
is public, so the checkable half of the convention is enforced rather than
asked for.
"""
import pytest

from adapters import openspec

_ADAPTERS = (openspec,)


def _public_symbols():
    return [
        (f"{module.__name__}.build_graph", module.build_graph)
        for module in _ADAPTERS
    ]


@pytest.mark.parametrize("name,obj", _public_symbols(), ids=lambda v: v)
def test_public_symbol_has_docstring(name, obj):
    doc = (obj.__doc__ or "").strip()
    assert doc, f"public symbol '{name}' has no docstring"


@pytest.mark.parametrize("name,obj", _public_symbols(), ids=lambda v: v)
def test_public_docstring_opens_with_a_sentence(name, obj):
    """First word is prose, not a bare identifier lifted from the code."""
    doc = (obj.__doc__ or "").strip()
    assert doc[:1].isupper(), (
        f"docstring for '{name}' should start with a capital, got {doc[:40]!r}"
    )
