package beta

import "testing"

var r = '\''
var s = "func TestInString(t *testing.T) {"

func TestBeta(t *testing.T) {
	if r == 0 || s == "" {
		t.Fatal("unreachable")
	}
}

func TestBetaTable(t *testing.T) {
	for _, tc := range []struct{ name string }{{"a"}, {"b"}} {
		t.Run(tc.name, func(t *testing.T) {})
	}
}
