package main

import (
	"encoding/json"
	"os"
	"reflect"
	"testing"
)

// TestFixtureDocument builds the shared fixture with the configuration
// ../profile.yaml gives and compares the result with ../expected.json. In a copy,
// point it at your own register and its expected document.
func TestFixtureDocument(t *testing.T) {
	got, err := json.Marshal(build("../fixture", "register", ".reg"))
	if err != nil {
		t.Fatal(err)
	}
	want, err := os.ReadFile("../expected.json")
	if err != nil {
		t.Fatal(err)
	}
	var gotValue, wantValue any
	if err := json.Unmarshal(got, &gotValue); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(want, &wantValue); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(gotValue, wantValue) {
		t.Errorf("document differs from ../expected.json:\n%s", got)
	}
}
