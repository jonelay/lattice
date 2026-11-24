// Command lines-adapter is a template Lattice adapter: it reads line registers and
// prints an interface document.
//
// Usage: lines-adapter --profile <resolved-profile.json> --target <dir>
//
// The register is the files in one directory whose names end in one extension, each
// line a declaration:
//
//	<kind> <id> [<edge-kind> <target-id>]... : <text>
//
// Copy this module and change readLine for your own format. The contract it follows is
// docs/adapters.md, "Writing your own adapter". Standard library only.
package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"unicode"
	"unicode/utf8"
)

const interfaceVersion = "1.4"

const declaration = `"<kind> <id> [<edge-kind> <target-id>]... : <text>"`

// coreKeys are the keys of the profile's adapter block that belong to the core; an
// adapter accepts them.
var coreKeys = map[string]bool{"native": true, "timeout_seconds": true, "command_timeout_seconds": true}

type provenance struct {
	File string `json:"file"`
	Line int    `json:"line"`
}

type node struct {
	ID         string            `json:"id"`
	Kind       string            `json:"kind"`
	Attrs      map[string]string `json:"attrs"`
	Provenance provenance        `json:"provenance"`
}

type edge struct {
	Src        string     `json:"src"`
	Tgt        string     `json:"tgt"`
	Kind       string     `json:"kind"`
	Provenance provenance `json:"provenance"`
	DeclaredBy int        `json:"declared_by"`
}

type finding struct {
	Severity   string     `json:"severity"`
	Code       string     `json:"code"`
	Message    string     `json:"message"`
	Provenance provenance `json:"provenance"`
	NodeID     *string    `json:"node_id"` // null: these findings are about input, not a node
}

// document is the interface document, built in source order.
type document struct {
	InterfaceVersion string    `json:"interface_version"`
	Nodes            []node    `json:"nodes"`
	Edges            []edge    `json:"edges"`
	Pathways         []any     `json:"pathways"`
	Findings         []finding `json:"findings"`
}

func newDocument() *document {
	return &document{InterfaceVersion: interfaceVersion, Nodes: []node{}, Edges: []edge{}, Pathways: []any{}, Findings: []finding{}}
}

// report records input the adapter could not read; never skip it in silence.
func (d *document) report(code, message, file string, line int, severity string) {
	d.Findings = append(d.Findings, finding{severity, code, message, provenance{file, line}, nil})
}

func (d *document) parseError(message, file string, line int) {
	d.report("PARSE_ERROR", message, file, line, "error")
}

// fail exits 2 with no document: the core then reports the adapter failed to run.
func fail(format string, args ...any) {
	fmt.Fprintf(os.Stderr, "lines-adapter: "+format+"\n", args...)
	os.Exit(2)
}

func parseArgs(args []string) (profile, target string) {
	values := map[string]string{}
	for i := 0; i < len(args); i += 2 {
		flag := args[i]
		if (flag != "--profile" && flag != "--target") || i+1 >= len(args) {
			fail("usage: --profile <resolved-profile.json> --target <dir>; unexpected %q", flag)
		}
		values[flag] = args[i+1]
	}
	profile, okProfile := values["--profile"]
	target, okTarget := values["--target"]
	if !okProfile || !okTarget {
		fail("usage: --profile <resolved-profile.json> --target <dir>")
	}
	return profile, target
}

// loadConfig returns the directory and extension from the resolved profile's adapter block.
func loadConfig(path string) (directory, extension string) {
	data, err := os.ReadFile(path)
	if err != nil {
		fail("cannot read profile %s: %v", path, err)
	}
	var profile map[string]any
	if err := json.Unmarshal(data, &profile); err != nil || profile["resolved_schema"] != "1" {
		fail("%s is not a resolved profile; pass the file `latt show` prints", path)
	}
	adapter, ok := profile["adapter"].(map[string]any)
	if !ok {
		fail("%s: the adapter block is missing", path)
	}
	for key := range adapter {
		if !coreKeys[key] && key != "directory" && key != "extension" {
			fail("%s: unknown adapter key %q", path, key)
		}
	}
	directory, okDirectory := adapter["directory"].(string)
	if !okDirectory {
		fail("%s: adapter key %q must be a string", path, "directory")
	}
	extension, okExtension := adapter["extension"].(string)
	if !okExtension {
		fail("%s: adapter key %q must be a string", path, "extension")
	}
	return directory, extension
}

// invalidByte returns the index of the first byte that is not valid UTF-8, or -1.
func invalidByte(raw []byte) int {
	for i := 0; i < len(raw); {
		r, size := utf8.DecodeRune(raw[i:])
		if r == utf8.RuneError && size == 1 {
			return i
		}
		i += size
	}
	return -1
}

// readLine reads one line of a register file into the document.
func readLine(d *document, raw []byte, file string, number int) {
	if at := invalidByte(raw); at >= 0 {
		d.parseError(fmt.Sprintf("line is not valid UTF-8 at byte %d, so it was not read", at+1), file, number)
		return
	}
	line := string(raw)
	if strings.TrimSpace(line) == "" || strings.HasPrefix(strings.TrimLeftFunc(line, unicode.IsSpace), "#") {
		return
	}
	head, text, colon := strings.Cut(line, ":")
	words := strings.Fields(head)
	if !colon || len(words) < 2 || len(words)%2 != 0 {
		d.parseError("line is not a declaration "+declaration+", so it was not read", file, number)
		return
	}
	id, kind := words[1], words[0]
	index := len(d.Nodes)
	d.Nodes = append(d.Nodes, node{id, kind, map[string]string{"text": strings.TrimSpace(text)}, provenance{file, number}})
	for i := 2; i+1 < len(words); i += 2 {
		d.Edges = append(d.Edges, edge{id, words[i+1], words[i], provenance{file, number}, index})
	}
}

func readFile(d *document, path, file string) {
	data, err := os.ReadFile(path)
	if err != nil {
		var pathError *os.PathError
		if errors.As(err, &pathError) {
			err = pathError.Err
		}
		d.parseError("file could not be read: "+err.Error(), file, 0)
		return
	}
	data = bytes.TrimPrefix(data, []byte("\xef\xbb\xbf"))
	for i, raw := range bytes.Split(data, []byte("\n")) {
		readLine(d, bytes.TrimSuffix(raw, []byte("\r")), file, i+1)
	}
}

func build(target, directory, extension string) *document {
	d := newDocument()
	root := filepath.Join(target, directory)
	entries, err := os.ReadDir(root) // sorted by name
	if err != nil {
		d.parseError(`directory "`+directory+`" does not exist or is not a directory`, directory, 0)
		return d
	}
	selected := 0
	for _, entry := range entries {
		name := entry.Name()
		if !utf8.ValidString(name) {
			d.parseError("a file name in this directory is not valid UTF-8, so it was not read", directory, 0)
			continue
		}
		if !strings.HasSuffix(name, extension) {
			continue
		}
		selected++
		file := directory + "/" + name
		if !entry.Type().IsRegular() {
			d.parseError("not a regular file, so it was not read", file, 0)
			continue
		}
		readFile(d, filepath.Join(root, name), file)
	}
	if selected == 0 {
		d.report("NO_MATCHING_FILES", `no file in "`+directory+`" ends in "`+extension+`"`, directory, 0, "info")
	}
	return d
}

func main() {
	profile, target := parseArgs(os.Args[1:])
	directory, extension := loadConfig(profile)
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(build(target, directory, extension)); err != nil {
		fail("cannot write the document: %v", err)
	}
}
