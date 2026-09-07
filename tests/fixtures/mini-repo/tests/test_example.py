import pytest


@pytest.mark.req("REQ-0101")
def test_basic_parse():
    assert True


@pytest.mark.req("REQ-0102")
class TestSpecGoals:
    def test_heading_parse(self):
        assert True


@pytest.mark.parametrize("value", [1, 2])
def test_unmarked_parametrized(value):
    """Sweep both example values without a marker."""
    assert value > 0


def test_unmarked_bare():
    assert True
