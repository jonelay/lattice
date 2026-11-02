# Fixture test file in the Python dialect. Excluded from collection via
# norecursedirs; the adapter reads it as text.


def test_uncited():
    pass


class TestGrouped:
    # A method reusing a name the file declares at top level: only the class
    # qualifier keeps the two apart.
    def test_beta_state(self):
        pass


# Requirement: beta/Beta holds state
def test_beta_state():
    pass


# Requirement: No such requirement
def test_dangling():
    pass


# Requirement: alpha/Alpha parses input / alpha/Alpha reports errors
def test_compound():
    pass
