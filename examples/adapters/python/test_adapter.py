"""Build the shared fixture and compare the document with expected.json.

    python3 -m unittest discover examples/adapters/python

The configuration is the one ../profile.yaml gives. In a copy, point the test at your own
register and its expected document.
"""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import adapter  # noqa: E402  (the template beside this file)

EXAMPLES = HERE.parent


class AdapterTest(unittest.TestCase):
    def test_fixture_document(self) -> None:
        document = adapter.build(str(EXAMPLES / "fixture"), "register", ".reg").to_json()
        # Round-trip through JSON, as the core reads it.
        printed = json.loads(json.dumps(document))
        self.assertEqual(printed, json.loads((EXAMPLES / "expected.json").read_text()))


if __name__ == "__main__":
    unittest.main()
