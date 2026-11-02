"""def test_old_put(): the docstring remembers a renamed test."""


def test_put():
    assert True


class TestGroup:
    def test_get(self, case):
        assert case


class Outer:
    class Inner:
        def test_deep(self):
            assert True
