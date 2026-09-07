"""The suggestion sidecar: inputs, legal pairings, ranking, offline default."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

from adapters import lattice_suggest
from adapters._profile import load_profile
from tests.conftest import resolve_profile

REPO = Path(__file__).resolve().parent.parent
PROGRAM = REPO / "adapters" / "lattice-suggest"

PROFILE_YAML = """
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\\\d+$"
    summary_attr: text
    attrs:
      text: {type: string}
  test:
    id_pattern: "^T-.+$"
    summary_attr: function
    attrs:
      function: {type: string}
  spec:
    id_pattern: "^S-.+$"
    summary_attr: title
    attrs:
      title: {type: string}
edge_kinds:
  verifies:
    allowed: [[test, req]]
"""


def node(node_id, kind, attrs):
    return {
        "id": node_id,
        "kind": kind,
        "attrs": attrs,
        "provenance": {"file": "f.py", "line": 1},
    }


CONTRACT = {
    "contract_version": "1.1",
    "nodes": [
        node("REQ-1", "req", {"text": "the motor shall report rotor temperature"}),
        node("REQ-2", "req", {"text": "the dashboard shall render a torque curve"}),
        node("T-marked", "test", {"function": "test_rotor_temp"}),
        node(
            "T-temp",
            "test",
            {
                "function": "test_rotor_temperature_reported",
                "docstring": "Rotor temperature is reported by the motor.",
            },
        ),
        node("S-1", "spec", {"title": "rotor thermal model"}),
    ],
    "edges": [
        {
            "src": "T-marked",
            "tgt": "REQ-1",
            "kind": "verifies",
            "provenance": {"file": "f.py", "line": 1},
        }
    ],
    "issues": [],
}


@pytest.fixture
def profile(tmp_path):
    path = tmp_path / "p.yaml"
    path.write_text(PROFILE_YAML)
    return load_profile(resolve_profile(path, tmp_path))


@pytest.fixture
def contract(tmp_path):
    path = tmp_path / "contract.json"
    path.write_text(json.dumps(CONTRACT))
    return path


def run(args, stdin=None):
    return subprocess.run(
        [sys.executable, "-m", "adapters.lattice_suggest", *args],
        capture_output=True,
        text=True,
        cwd=REPO,
        input=stdin,
    )


# Requirement: The sidecar is a program emitting a suggestion document


def test_program_emits_a_document_on_stdout(tmp_path, contract):
    path = tmp_path / "p.yaml"
    path.write_text(PROFILE_YAML)
    resolved = resolve_profile(path, tmp_path)

    result = run(
        ["--profile", str(resolved), "--contract", str(contract), "--edge-kind", "verifies"]
    )

    assert result.returncode == 0, result.stderr
    document = json.loads(result.stdout)
    assert document["suggestion_version"] == "1.0"
    assert document["producer"].startswith("lattice-suggest")
    assert document["suggestions"] == []


def test_program_reads_a_contract_document_from_stdin(tmp_path):
    path = tmp_path / "p.yaml"
    path.write_text(PROFILE_YAML)
    resolved = resolve_profile(path, tmp_path)

    result = run(
        ["--profile", str(resolved), "--edge-kind", "verifies"],
        stdin=json.dumps(CONTRACT),
    )

    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["suggestions"] == []


def test_program_reports_an_unreadable_contract_rather_than_crashing(tmp_path):
    path = tmp_path / "p.yaml"
    path.write_text(PROFILE_YAML)
    resolved = resolve_profile(path, tmp_path)

    result = run(["--profile", str(resolved), "--edge-kind", "verifies"], stdin="{not json")

    assert result.returncode == 2
    assert "contract document" in result.stderr


def test_program_reports_an_undeclared_edge_kind(tmp_path, contract):
    path = tmp_path / "p.yaml"
    path.write_text(PROFILE_YAML)
    resolved = resolve_profile(path, tmp_path)

    result = run(
        ["--profile", str(resolved), "--contract", str(contract), "--edge-kind", "mitigates"]
    )

    assert result.returncode == 2
    assert "mitigates" in result.stderr
    assert "verifies" in result.stderr


# Requirement: Candidates are constrained to profile-legal pairings


def test_constrained_to_the_unattributed_population(profile):
    sources, _ = lattice_suggest.candidates(CONTRACT, profile, "verifies")

    # T-marked already carries a verifies edge, so it is not a candidate source.
    assert [n["id"] for n in sources] == ["T-temp"]


def test_constrained_targets_exclude_forbidden_kinds(profile):
    _, targets = lattice_suggest.candidates(CONTRACT, profile, "verifies")

    assert {n["id"] for n in targets} == {"REQ-1", "REQ-2"}
    assert all(n["kind"] == "req" for n in targets), "spec is not a legal verifies target"


def test_a_forbidden_pairing_is_never_proposed(profile):
    # The spec node's title is the closest text to the test's docstring, but
    # (test, spec) is not an allowed verifies pairing.
    suggestions = lattice_suggest.rank(
        *lattice_suggest.candidates(CONTRACT, profile, "verifies"),
        profile,
        "verifies",
        _fake_embed,
        "fake",
        top_k=5,
    )

    assert suggestions, "the fixture should produce candidates"
    assert all(s["tgt"].startswith("REQ-") for s in suggestions)


def test_the_sidecar_carries_no_vocabulary_of_its_own(tmp_path):
    """A profile naming its kinds differently changes nothing but the profile.

    The sidecar names no kind, no ID shape and no edge kind of its own; it reads
    all three from the profile, so a register in another vocabulary works with
    no change here.
    """
    renamed = PROFILE_YAML.replace("req:", "obligation:").replace(
        "test:", "check:"
    ).replace("[[test, req]]", "[[check, obligation]]")
    path = tmp_path / "renamed.yaml"
    path.write_text(renamed)
    profile = load_profile(resolve_profile(path, tmp_path))

    document = json.loads(
        json.dumps(CONTRACT).replace('"kind": "req"', '"kind": "obligation"').replace(
            '"kind": "test"', '"kind": "check"'
        )
    )
    sources, targets = lattice_suggest.candidates(document, profile, "verifies")

    assert [n["id"] for n in sources] == ["T-temp"]
    assert {n["id"] for n in targets} == {"REQ-1", "REQ-2"}


# Requirement: Ranking baseline is embedding similarity over declared text


def _fake_embed(texts: list[str]) -> list[list[float]]:
    """A deterministic stand-in: one dimension per vocabulary word.

    Not a quality model and not meant to be. It makes the ranking assertions
    depend on the sidecar's own logic rather than on a network service.
    """
    vocabulary = sorted({word for text in texts for word in _words(text)})
    return [
        [1.0 if word in _words(text) else 0.0 for word in vocabulary] for text in texts
    ]


def _words(text: str) -> set[str]:
    return {w.strip(".,").lower() for w in text.replace("_", " ").split() if w}


def test_rank_puts_the_more_similar_requirement_first(profile):
    sources, targets = lattice_suggest.candidates(CONTRACT, profile, "verifies")
    suggestions = lattice_suggest.rank(
        sources, targets, profile, "verifies", _fake_embed, "fake", top_k=5
    )

    first = [s for s in suggestions if s["src"] == "T-temp"][0]
    assert first["tgt"] == "REQ-1", suggestions


def test_rank_records_the_model_in_every_basis(profile):
    sources, targets = lattice_suggest.candidates(CONTRACT, profile, "verifies")
    suggestions = lattice_suggest.rank(
        sources, targets, profile, "verifies", _fake_embed, "fake-model-1", top_k=5
    )

    assert suggestions
    assert all("fake-model-1" in s["basis"] for s in suggestions)
    assert all("cosine" in s["basis"] for s in suggestions)


def test_rank_orders_by_descending_score_with_a_stable_tie_break(profile):
    sources, targets = lattice_suggest.candidates(CONTRACT, profile, "verifies")
    suggestions = lattice_suggest.rank(
        sources, targets, profile, "verifies", _fake_embed, "fake", top_k=5
    )

    keys = [(-s["score"], s["src"], s["tgt"], s["kind"]) for s in suggestions]
    assert keys == sorted(keys)


TEXT_ATTRS_YAML = """
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\\\d+$"
    summary_attr: text
    text_attrs: [text, body]
    attrs:
      text: {type: string}
      body: {type: string}
  test:
    id_pattern: "^T-.+$"
    summary_attr: function
    attrs:
      function: {type: string}
  spec:
    id_pattern: "^S-.+$"
    summary_attr: title
    text_attrs: []
    attrs:
      title: {type: string}
edge_kinds:
  verifies:
    allowed: [[test, req]]
"""


@pytest.fixture
def text_attrs_profile(tmp_path):
    path = tmp_path / "ta.yaml"
    path.write_text(TEXT_ATTRS_YAML)
    return load_profile(resolve_profile(path, tmp_path))


def test_rank_reads_the_profile_declared_text_attrs(text_attrs_profile):
    """The profile names the rankable attrs, so a second one needs no code
    change — and the sidecar names no attr of its own."""
    texts = lattice_suggest._texts(
        node("REQ-1", "req", {"text": "the title", "body": "the prose"}),
        text_attrs_profile,
    )

    assert texts == ["the title", "the prose"]


def test_rank_falls_back_to_summary_attr_when_text_attrs_is_absent(text_attrs_profile):
    """A profile written before `text_attrs` existed keeps its ranked text."""
    texts = lattice_suggest._texts(
        node("T-x", "test", {"function": "test_x"}), text_attrs_profile
    )

    assert texts == ["test_x"]


def test_rank_treats_declared_empty_text_attrs_as_textless(text_attrs_profile):
    """`[]` is a declaration that the kind offers nothing to rank, which is a
    different answer from an absent key — it must not fall back."""
    texts = lattice_suggest._texts(
        node("S-x", "spec", {"title": "a title"}), text_attrs_profile
    )

    assert texts == []


def test_rank_is_deterministic_for_a_fixed_backend(profile):
    sources, targets = lattice_suggest.candidates(CONTRACT, profile, "verifies")
    args = (sources, targets, profile, "verifies", _fake_embed, "fake")

    assert lattice_suggest.rank(*args, top_k=5) == lattice_suggest.rank(*args, top_k=5)


# Requirement: The sidecar runs offline by default


def test_offline_default_emits_an_empty_document_and_says_so(tmp_path, contract):
    path = tmp_path / "p.yaml"
    path.write_text(PROFILE_YAML)
    resolved = resolve_profile(path, tmp_path)

    result = run(
        ["--profile", str(resolved), "--contract", str(contract), "--edge-kind", "verifies"]
    )

    assert result.returncode == 0
    assert json.loads(result.stdout)["suggestions"] == []
    # Distinguishable from a backend that ran and found nothing.
    assert "no backend configured" in result.stderr
    assert "1 unattributed" in result.stderr


def test_offline_default_touches_no_network(monkeypatch, profile):
    """The default path must not import or reach a network at all."""
    import urllib.request

    def forbidden(*args, **kwargs):
        raise AssertionError("the offline path opened a connection")

    monkeypatch.setattr(urllib.request, "urlopen", forbidden)
    sources, _ = lattice_suggest.candidates(CONTRACT, profile, "verifies")

    assert sources  # candidates are computed without a backend


def test_the_offline_backend_refuses_rather_than_returning_zeros():
    with pytest.raises(lattice_suggest.SuggestError, match="no embedding backend"):
        lattice_suggest.offline_backend(["a"])


def test_an_unreachable_ollama_host_is_an_error_not_a_silent_empty_document():
    embed = lattice_suggest.ollama_backend("http://127.0.0.1:9", "m")

    with pytest.raises(lattice_suggest.SuggestError, match="unreachable"):
        embed(["a"])


# Requirement: A node with no text is reported and excluded, never silently ranked


def test_a_textless_source_is_excluded_and_reported(profile, capsys):
    document = json.loads(json.dumps(CONTRACT))
    document["nodes"].append(node("T-notext", "test", {}))
    sources, targets = lattice_suggest.candidates(document, profile, "verifies")
    received: list[str] = []

    def embed(texts):
        received.extend(texts)
        return _fake_embed(texts)

    suggestions = lattice_suggest.rank(
        sources, targets, profile, "verifies", embed, "fake", top_k=5
    )

    assert suggestions, "the textful source must still be ranked"
    assert all(s["src"] != "T-notext" for s in suggestions)
    assert "" not in received, "a textless node's text must never reach the backend"
    err = capsys.readouterr().err
    assert "1 source(s)" in err
    assert "no text" in err


def test_an_all_textless_population_makes_no_backend_call(profile, capsys):
    document = json.loads(json.dumps(CONTRACT))
    document["nodes"] = [
        node("REQ-1", "req", {"text": "the motor shall report rotor temperature"}),
        node("T-a", "test", {}),
        node("T-b", "test", {}),
    ]
    document["edges"] = []

    def embed(texts):
        raise AssertionError("the backend was invoked for an all-textless population")

    suggestions = lattice_suggest.rank(
        *lattice_suggest.candidates(document, profile, "verifies"),
        profile,
        "verifies",
        embed,
        "fake",
        top_k=5,
    )

    assert suggestions == []
    assert "no text" in capsys.readouterr().err


def test_program_with_an_all_textless_population_never_needs_the_network(tmp_path):
    """The S35 finding: a textless register produced 4520 suggestions all at
    cosine 1.0 in silence. Excluded before the embed call, it needs no backend."""
    path = tmp_path / "p.yaml"
    path.write_text(PROFILE_YAML)
    resolved = resolve_profile(path, tmp_path)
    document = json.loads(json.dumps(CONTRACT))
    document["nodes"] = [node("REQ-1", "req", {"text": "t"}), node("T-a", "test", {})]
    document["edges"] = []

    result = run(
        [
            "--profile", str(resolved), "--edge-kind", "verifies",
            "--backend", "ollama", "--host", "http://127.0.0.1:9",
        ],
        stdin=json.dumps(document),
    )

    assert result.returncode == 0, result.stderr
    assert json.loads(result.stdout)["suggestions"] == []
    assert "no text" in result.stderr


# Requirement: Ranking baseline is embedding similarity over declared text
# (the chunking scenarios)

CHUNK_YAML = """
name: t
profile_version: "1.0.0"
node_kinds:
  req:
    id_pattern: "^REQ-\\\\d+$"
    summary_attr: text
    text_attrs: [text, body]
    text_chunk_line_prefix: "#### Scenario:"
    attrs:
      text: {type: string}
      body: {type: string}
  test:
    id_pattern: "^T-.+$"
    summary_attr: function
    text_attrs: [function]
    text_chunk_line_prefix: "#### Scenario:"
    attrs:
      function: {type: string}
edge_kinds:
  verifies:
    allowed: [[test, req]]
"""


@pytest.fixture
def chunk_profile(tmp_path):
    path = tmp_path / "chunk.yaml"
    path.write_text(CHUNK_YAML)
    return load_profile(resolve_profile(path, tmp_path))


def _recording_embed(seen):
    """A backend that records the exact batch it was handed.

    The batch is the gate: it is what the sidecar decided to rank, before any
    similarity arithmetic can obscure a splitting or ordering mistake.
    """

    def embed(texts):
        seen.append(list(texts))
        return _fake_embed(texts)

    return embed


def test_chunks_splits_at_every_line_carrying_the_prefix():
    text = "preamble prose\n#### Scenario: first\nalpha\n#### Scenario: second\nbeta"

    assert lattice_suggest._chunks(text, "#### Scenario:") == [
        "preamble prose\n",
        "#### Scenario: first\nalpha\n",
        "#### Scenario: second\nbeta",
    ]


def test_chunks_partition_the_text_without_dropping_any_of_it():
    """The split partitions, never extracts.

    A chunk silently lost is invisible downstream — nothing can tell a missing
    region from one that merely scored low — which is the third invariant's
    failure mode. The sentinel words would vanish under a split that consumed
    its delimiter or dropped the preamble.
    """
    text = (
        "SENTINELHEAD prose\n"
        "#### Scenario: one\nSENTINELONE\n"
        "#### Scenario: two\nSENTINELTWO"
    )

    parts = lattice_suggest._chunks(text, "#### Scenario:")

    assert "".join(parts) == text
    for sentinel in ("SENTINELHEAD", "SENTINELONE", "SENTINELTWO"):
        assert any(sentinel in part for part in parts), sentinel


def test_chunks_leading_marker_yields_no_empty_head():
    text = "#### Scenario: only\nalpha"

    assert lattice_suggest._chunks(text, "#### Scenario:") == [text]


def test_chunks_text_without_the_marker_stays_whole():
    text = "prose with no marker at all"

    assert lattice_suggest._chunks(text, "#### Scenario:") == [text]


def test_chunks_marker_must_start_the_line():
    """A literal prefix, matched at line start — an indented marker is prose."""
    text = "head\n    #### Scenario: indented\nalpha"

    assert lattice_suggest._chunks(text, "#### Scenario:") == [text]


def test_rank_sends_one_text_per_target_chunk(chunk_profile):
    seen = []
    document = {
        "contract_version": "1.1",
        "nodes": [
            node("REQ-1", "req", {"text": "title", "body": "pre\n#### Scenario: a\nx"}),
            node("T-a", "test", {"function": "test_a"}),
        ],
        "edges": [],
    }
    sources, targets = lattice_suggest.candidates(document, chunk_profile, "verifies")
    lattice_suggest.rank(
        sources, targets, chunk_profile, "verifies", _recording_embed(seen), "fake", 5
    )

    assert seen[0] == ["test_a", "title \npre\n", "#### Scenario: a\nx"]


def test_rank_never_chunks_a_source(chunk_profile):
    """Chunking is a property of the ranked role, not of the kind.

    The `test` kind declares a prefix here and supplies the sources; a source
    carrying the marker must still be embedded as one text, because pooling
    happens on the target side.
    """
    seen = []
    document = {
        "contract_version": "1.1",
        "nodes": [
            node("REQ-1", "req", {"text": "title"}),
            node("T-a", "test", {"function": "one\n#### Scenario: b\ntwo"}),
        ],
        "edges": [],
    }
    sources, targets = lattice_suggest.candidates(document, chunk_profile, "verifies")
    lattice_suggest.rank(
        sources, targets, chunk_profile, "verifies", _recording_embed(seen), "fake", 5
    )

    assert seen[0][0] == "one\n#### Scenario: b\ntwo"


def test_rank_scores_a_pairing_as_the_targets_best_chunk(chunk_profile):
    """Max-pooling, not mean: the requirement wins on its matching block.

    REQ-1's second scenario is the test's subject, and the rest of its body is
    unrelated. Under one vector, or under mean pooling, the noise outweighs the
    match and the shorter REQ-2 wins.
    """
    document = {
        "contract_version": "1.1",
        "nodes": [
            node(
                "REQ-1",
                "req",
                {
                    "text": "motor",
                    "body": (
                        "#### Scenario: dashboard renders a torque curve\n"
                        "dashboard torque curve rendering widget\n"
                        "#### Scenario: rotor temperature is reported\n"
                        "rotor temperature reported"
                    ),
                },
            ),
            node("REQ-2", "req", {"text": "rotor", "body": "temperature alpha beta"}),
            node("T-a", "test", {"function": "rotor temperature reported"}),
        ],
        "edges": [],
    }
    sources, targets = lattice_suggest.candidates(document, chunk_profile, "verifies")
    suggestions = lattice_suggest.rank(
        sources, targets, chunk_profile, "verifies", _fake_embed, "fake", 5
    )

    assert suggestions[0]["tgt"] == "REQ-1"


def test_rank_without_a_declared_prefix_is_unchanged(text_attrs_profile):
    """A profile written before this key ranks exactly as it did — the batch
    it hands the backend is the one undivided text per node it always was."""
    seen = []
    document = {
        "contract_version": "1.1",
        "nodes": [
            node("REQ-1", "req", {"text": "title", "body": "pre\n#### Scenario: a\nx"}),
            node("T-a", "test", {"function": "test_a"}),
        ],
        "edges": [],
    }
    sources, targets = lattice_suggest.candidates(
        document, text_attrs_profile, "verifies"
    )
    lattice_suggest.rank(
        sources, targets, text_attrs_profile, "verifies",
        _recording_embed(seen), "fake", 5,
    )

    assert seen[0] == ["test_a", "title \npre\n#### Scenario: a\nx"]


def test_rank_reports_a_declared_prefix_that_never_splits(chunk_profile, capsys):
    """A marker absent from every node is a profile typo or an adapter that
    stopped emitting the body — a silent no-op reads as 'nothing to see here'."""
    document = {
        "contract_version": "1.1",
        "nodes": [
            node("REQ-1", "req", {"text": "title", "body": "no marker here"}),
            node("T-a", "test", {"function": "test_a"}),
        ],
        "edges": [],
    }
    sources, targets = lattice_suggest.candidates(document, chunk_profile, "verifies")
    lattice_suggest.rank(
        sources, targets, chunk_profile, "verifies", _fake_embed, "fake", 5
    )

    assert "text_chunk_line_prefix" in capsys.readouterr().err


HARNESS = REPO / "notes" / "chunking-s40" / "rank_chunked.py"

# The bodies the two splitters must agree on: the ordinary shape, a leading
# marker with no preamble, no marker at all, an indented marker that is prose,
# a blank line between blocks, and a trailing marker with an empty block.
SPLITTER_CASES = [
    "preamble\n#### Scenario: a\nalpha\n#### Scenario: b\nbeta",
    "#### Scenario: only\nalpha",
    "prose with no marker",
    "head\n    #### Scenario: indented\nalpha",
    "head\n\n#### Scenario: a\n\nalpha\n\n#### Scenario: b\nbeta",
    "head\n#### Scenario: a\nalpha\n#### Scenario: trailing",
]


@pytest.mark.skipif(not HARNESS.exists(), reason="measurement harness not in the tree")
@pytest.mark.parametrize("body", SPLITTER_CASES)
def test_shipped_splitter_reproduces_the_measured_one(chunk_profile, body):
    """The gate the measurement asked for: the shipped splitter *is* the one
    whose numbers are being claimed.

    `notes/chunking-s40/RESULTS.md` requires the ship change to gate on its own
    splitter rather than inherit the measured numbers. Because the marker moved
    into the profile rather than being generalised, that is satisfiable by
    reproduction — this compares the shipped chunker against `rank_chunked.py`'s
    `bare` mode, which produced `suggestions-bare-topk20.json`.
    """
    import importlib.util

    spec = importlib.util.spec_from_file_location("rank_chunked", HARNESS)
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)

    target = node("REQ-1", "req", {"text": "a title", "body": body})

    assert lattice_suggest._target_chunks(target, chunk_profile) == harness.chunks_for(
        target, chunk_profile, "bare"
    )


@pytest.mark.skipif(not HARNESS.exists(), reason="measurement harness not in the tree")
def test_the_embed_batch_matches_the_harness_batch(chunk_profile):
    """The whole batch, in order — not just the splitter on one node.

    Byte-identity against the pinned document is confirmation, but it cannot
    separate a real behaviour change from embedding noise: the backend's
    determinism is only established for a fixed batch. This is the hermetic
    form of that gate — sources in candidate order, then each target's chunks
    contiguously, exactly as `rank_chunked.py` assembled them.
    """
    import importlib.util

    spec = importlib.util.spec_from_file_location("rank_chunked", HARNESS)
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)

    document = {
        "contract_version": "1.1",
        "nodes": [
            node("T-a", "test", {"function": "test_alpha"}),
            node("T-b", "test", {"function": "test_beta"}),
            node("REQ-1", "req", {"text": "one", "body": SPLITTER_CASES[0]}),
            node("REQ-2", "req", {"text": "two", "body": SPLITTER_CASES[2]}),
            node("REQ-3", "req", {"text": "three", "body": SPLITTER_CASES[4]}),
        ],
        "edges": [],
    }
    sources, targets = lattice_suggest.candidates(document, chunk_profile, "verifies")

    seen = []
    lattice_suggest.rank(
        sources, targets, chunk_profile, "verifies", _recording_embed(seen), "fake", 5
    )

    expected = [" \n".join(lattice_suggest._texts(n, chunk_profile)) for n in sources]
    for target in targets:
        expected.extend(harness.chunks_for(target, chunk_profile, "bare"))

    assert seen[0] == expected


def test_chunks_cuts_on_newline_only_not_on_every_unicode_line_break():
    """`str.splitlines` would also cut at form feed, vertical tab and U+2028.

    The profile names a line prefix, and the register's lines are newline-
    separated; dividing a body at a character nobody declared would rank text
    the measured splitter kept together. A `splitlines`-based implementation
    passes every other test here, so this is the one that pins the choice.
    """
    text = "head #### Scenario: not a line start\nbody"

    assert lattice_suggest._chunks(text, "#### Scenario:") == [text]


# Requirement: Duplicate node IDs rank as one candidate


def test_a_duplicated_target_scores_its_first_occurrence_and_appears_once(profile):
    document = json.loads(json.dumps(CONTRACT))
    # First occurrence carries the text that matches the unattributed test;
    # the later occurrence is unrelated. Keying occurrences by ID kept only
    # the last one, so a matching first occurrence was never scored.
    document["nodes"].insert(
        0, node("REQ-3", "req", {"text": "rotor temperature is reported by the motor"})
    )
    document["nodes"].append(
        node("REQ-3", "req", {"text": "an unrelated pump curve rendering row"})
    )
    sources, targets = lattice_suggest.candidates(document, profile, "verifies")
    suggestions = lattice_suggest.rank(
        sources, targets, profile, "verifies", _fake_embed, "fake", top_k=5
    )

    for_temp = [s for s in suggestions if s["src"] == "T-temp"]
    assert [s["tgt"] for s in for_temp].count("REQ-3") == 1
    assert for_temp[0]["tgt"] == "REQ-3", for_temp


def test_a_duplicated_source_is_ranked_once_over_its_union_of_text(profile, capsys):
    document = json.loads(json.dumps(CONTRACT))
    # One occurrence carries text, the other none: the union ranks, and the
    # textless occurrence neither excludes the ID nor doubles its output.
    document["nodes"].append(
        node("T-dup", "test", {"function": "test_motor_reports_rotor_temperature"})
    )
    document["nodes"].append(node("T-dup", "test", {}))
    sources, targets = lattice_suggest.candidates(document, profile, "verifies")
    suggestions = lattice_suggest.rank(
        sources, targets, profile, "verifies", _fake_embed, "fake", top_k=5
    )

    for_dup = [s for s in suggestions if s["src"] == "T-dup"]
    assert for_dup, "the union of occurrence text must rank"
    tgts = [s["tgt"] for s in for_dup]
    assert len(tgts) == len(set(tgts)), tgts
    assert for_dup[0]["tgt"] == "REQ-1", for_dup
    err = capsys.readouterr().err
    assert "excluded" not in err, "the union must not be excluded as textless"


def test_merged_duplicates_are_reported_on_stderr(profile, capsys):
    document = json.loads(json.dumps(CONTRACT))
    document["nodes"].append(
        node("REQ-2", "req", {"text": "a second torque curve row"})
    )
    sources, targets = lattice_suggest.candidates(document, profile, "verifies")
    lattice_suggest.rank(
        sources, targets, profile, "verifies", _fake_embed, "fake", top_k=5
    )

    err = capsys.readouterr().err
    assert "duplicate" in err
    assert "1" in err
