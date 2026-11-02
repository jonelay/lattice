package alpha

import (
	"strings"
	"testing"
)

// func TestInComment(t *testing.T) {}

/* func TestInBlockComment(t *testing.T) {} */

var source = `
func TestInRawString(t *testing.T) {
}
`

type Suite struct{}

func (s *Suite) TestMethod(t *testing.T) {}

func helper[T any](v T) T { return v }

func TestPlain(t *testing.T) {
	t.Run("TestSubtest", func(t *testing.T) {
		_ = strings.Contains(source, "{")
	})
}

func TestGeneric(t *testing.T) { _ = helper[int](1) }

func BenchmarkPlain(b *testing.B) {}

func FuzzPlain(f *testing.F) { f.Fuzz(func(t *testing.T, s string) {}) }

func ExamplePlain() {
	// Output:
}
