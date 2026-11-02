use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::{ScanError, Scope, beyond_lines, check_sources, go, python, rust, validate_id_names};
use crate::{Document, Issue, Node, Provenance, Severity};

/// A scratch target directory, removed on drop.
struct Target(PathBuf);

impl Target {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "latt-adapter-sources-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let dir = root.join("target");
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, path: &str, contents: &str) -> &Self {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
        self
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn kinds(form: Value) -> BTreeMap<String, Value> {
    [
        ("test".to_owned(), json!({"id_names": form})),
        ("plain".to_owned(), json!({})),
    ]
    .into_iter()
    .collect()
}

/// Run the check over one node per ID, all of kind `test`, declared at
/// `REGISTER.md` lines 1, 2, ...
fn check(target: &Target, form: Value, ids: &[&str]) -> Vec<Issue> {
    let mut document = Document::new();
    for (line, id) in ids.iter().enumerate() {
        document.nodes.push(Node::new(
            *id,
            "test",
            Provenance::new("REGISTER.md", u32::try_from(line).unwrap() + 1),
        ));
    }
    check_sources(&mut document, &kinds(form), target.path()).unwrap();
    document.findings
}

fn messages(findings: &[Issue]) -> Vec<String> {
    findings.iter().map(|f| f.message.clone()).collect()
}

fn reason(target: &Target, form: Value, id: &str) -> Option<String> {
    let findings = check(target, form, &[id]);
    assert!(findings.len() <= 1, "{findings:?}");
    findings.first().map(|finding| {
        assert_eq!(finding.code, "SOURCE_MISSING");
        let prefix = format!("'{id}' does not resolve: ");
        finding
            .message
            .strip_prefix(&prefix)
            .unwrap_or_else(|| panic!("{}", finding.message))
            .to_owned()
    })
}

// --- forms --------------------------------------------------------------

#[test]
fn source_forms_accept_presets_and_blocks() {
    for value in [
        json!("file"),
        json!("cargo"),
        json!("pytest"),
        json!("gotest"),
        json!({"language": "rust", "separator": "::"}),
        json!({"language": "python", "separator": "::", "params": "[]"}),
    ] {
        validate_id_names(&value).unwrap_or_else(|error| panic!("{value}: {error}"));
    }
}

#[test]
fn source_forms_reject_with_what_is_accepted() {
    let cases = [
        (
            json!("jest"),
            vec!["'jest'", "'file', 'cargo', 'pytest' and 'gotest'"],
        ),
        (
            json!({"language": "ruby", "separator": "::"}),
            vec!["'ruby'", "'rust', 'python' and 'go'"],
        ),
        (json!({"language": "python"}), vec!["'separator'"]),
        (
            json!({"language": "python", "separator": ""}),
            vec!["'separator'"],
        ),
        (
            json!({"language": "python", "separator": "::", "params": "["}),
            vec!["'params'", "two characters"],
        ),
        (
            json!({"language": "rust", "separator": "::", "sep": "."}),
            vec!["'sep'", "'language', 'separator' and 'params'"],
        ),
        (json!(["file"]), vec!["preset name"]),
    ];
    for (value, needles) in cases {
        let error = validate_id_names(&value).unwrap_err();
        for needle in needles {
            assert!(error.contains(needle), "{value}: {error} lacks {needle}");
        }
    }
}

#[test]
fn source_check_errs_on_unknown_form_and_appends_nothing() {
    let target = Target::new();
    let mut document = Document::new();
    document
        .nodes
        .push(Node::new("a.md", "test", Provenance::new("R.md", 1)));
    let error = check_sources(&mut document, &kinds(json!("jest")), target.path()).unwrap_err();
    assert!(
        error.contains("node kind 'test'") && error.contains("'jest'"),
        "{error}"
    );
    assert!(document.findings.is_empty());
}

// --- file addresses -----------------------------------------------------

#[test]
fn source_file_present_is_silent_and_missing_is_reported_against_its_row() {
    let target = Target::new();
    target.write("docs/here.md", "");
    let findings = check(&target, json!("file"), &["docs/here.md", "docs/gone.md"]);
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code, "SOURCE_MISSING");
    assert_eq!(finding.severity, Severity::Warning);
    assert_eq!(finding.node_id.as_deref(), Some("docs/gone.md"));
    assert_eq!(finding.provenance, Provenance::new("REGISTER.md", 2));
    assert_eq!(
        finding.message,
        "'docs/gone.md' does not resolve: no file at 'docs/gone.md'"
    );
}

#[test]
fn source_file_outside_the_target_never_resolves() {
    let target = Target::new();
    std::fs::write(target.path().parent().unwrap().join("secrets.txt"), "").unwrap();
    assert_eq!(
        reason(&target, json!("file"), "../secrets.txt").as_deref(),
        Some("'../secrets.txt' is outside the target")
    );
    assert_eq!(
        reason(&target, json!("file"), "/etc/hostname").as_deref(),
        Some("'/etc/hostname' is outside the target")
    );
}

#[cfg(unix)]
#[test]
fn source_file_symlinks_escaping_or_dangling() {
    let target = Target::new();
    let outside = target.path().parent().unwrap().join("outside.md");
    std::fs::write(&outside, "").unwrap();
    target.write("docs/real.md", "");
    std::os::unix::fs::symlink(&outside, target.path().join("docs/link.md")).unwrap();
    std::os::unix::fs::symlink("real.md", target.path().join("docs/inside.md")).unwrap();
    std::os::unix::fs::symlink("nowhere.md", target.path().join("docs/dangling.md")).unwrap();
    assert_eq!(
        reason(&target, json!("file"), "docs/link.md").as_deref(),
        Some("'docs/link.md' is outside the target")
    );
    assert_eq!(reason(&target, json!("file"), "docs/inside.md"), None);
    assert_eq!(
        reason(&target, json!("file"), "docs/dangling.md").as_deref(),
        Some("no file at 'docs/dangling.md'")
    );
}

#[test]
fn source_file_directory_and_path_through_a_file() {
    let target = Target::new();
    target.write("docs/a.md", "");
    assert_eq!(
        reason(&target, json!("file"), "docs").as_deref(),
        Some("'docs' is not a regular file")
    );
    assert_eq!(
        reason(&target, json!("file"), "docs/a.md/b").as_deref(),
        Some("no file at 'docs/a.md/b'")
    );
}

#[cfg(unix)]
#[test]
fn source_file_unreadable_directory_could_not_be_read() {
    use std::os::unix::fs::PermissionsExt;
    let target = Target::new();
    target.write("locked/a.md", "");
    let locked = target.path().join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let bound = std::fs::read_dir(&locked).is_err();
    let got = reason(&target, json!("file"), "locked/a.md");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !bound {
        return; // running as root: permissions do not bind
    }
    let got = got.unwrap();
    assert!(
        got.starts_with("'locked/a.md' could not be read: "),
        "{got}"
    );
}

#[test]
fn source_null_id_names_reads_as_absent() {
    validate_id_names(&Value::Null).unwrap();
    let target = Target::new();
    assert!(check(&target, Value::Null, &["docs/gone.md"]).is_empty());
}

#[test]
fn source_kind_without_id_names_is_not_checked() {
    let target = Target::new();
    let mut document = Document::new();
    document.nodes.push(Node::new(
        "tests/gone.py::x",
        "plain",
        Provenance::new("R.md", 1),
    ));
    check_sources(&mut document, &kinds(json!("pytest")), target.path()).unwrap();
    assert!(document.findings.is_empty());
}

#[test]
fn source_findings_follow_existing_ones_in_node_order() {
    let target = Target::new();
    let mut document = Document::new();
    document.parse_error("earlier", "R.md", 9);
    for (line, id) in [(3, "b.md"), (1, "a.md")] {
        document
            .nodes
            .push(Node::new(id, "test", Provenance::new("R.md", line)));
    }
    check_sources(&mut document, &kinds(json!("file")), target.path()).unwrap();
    let ids: Vec<_> = document
        .findings
        .iter()
        .map(|f| f.node_id.as_deref())
        .collect();
    assert_eq!(ids, [None, Some("b.md"), Some("a.md")]);
}

// --- address grammar ----------------------------------------------------

#[test]
fn source_shape_checks_come_before_the_filesystem() {
    let target = Target::new();
    assert_eq!(
        reason(&target, json!("cargo"), "tests/foo.rs::::t").as_deref(),
        Some("empty name in node ID")
    );
    assert_eq!(
        reason(&target, json!("cargo"), "tests/foo.py::t").as_deref(),
        Some("a rust address names a '.rs' file")
    );
    assert_eq!(
        reason(&target, json!("pytest"), "tests/t.py::A::B::c").as_deref(),
        Some("python addresses reach at most 2 names")
    );
    assert_eq!(
        reason(&target, json!("gotest"), "pkg/s_test.go::TestPut::empty").as_deref(),
        Some("go addresses reach at most 1 names")
    );
    assert_eq!(
        reason(&target, json!("pytest"), "tests").as_deref(),
        Some("a python address names a '.py' file")
    );
}

#[test]
fn source_params_are_dropped_before_the_split() {
    let target = Target::new();
    target.write("tests/test_a.py", "def test_new(case):\n    pass\n");
    for id in [
        "tests/test_a.py::test_new[case-1]",
        "tests/test_a.py::test_new[a::b]",
        "tests/test_a.py",
    ] {
        assert_eq!(reason(&target, json!("pytest"), id), None, "{id}");
    }
    assert_eq!(
        reason(&target, json!("pytest"), "tests/test_a.py::test_old[x]").as_deref(),
        Some("'tests/test_a.py' defines no 'test_old'")
    );
}

#[test]
fn source_structured_block_with_its_own_separator() {
    let target = Target::new();
    target.write("src/lib.rs", "mod tests { fn splits() {} }\n");
    let form = json!({"language": "rust", "separator": "#"});
    assert_eq!(
        reason(&target, form.clone(), "src/lib.rs#tests#splits"),
        None
    );
    assert_eq!(
        reason(&target, form, "src/lib.rs#unit#splits").as_deref(),
        Some("'src/lib.rs' defines no module 'unit'")
    );
}

#[test]
fn source_missing_prefix_names_the_container() {
    let target = Target::new();
    target.write(
        "tests/test_a.py",
        "class TestGroup:\n    def test_b(self): pass\n",
    );
    assert_eq!(
        reason(
            &target,
            json!("pytest"),
            "tests/test_a.py::TestGone::test_b"
        )
        .as_deref(),
        Some("'tests/test_a.py' defines no class 'TestGone'")
    );
    assert_eq!(
        reason(
            &target,
            json!("pytest"),
            "tests/test_a.py::TestGroup::test_c"
        )
        .as_deref(),
        Some("'tests/test_a.py' defines no 'TestGroup::test_c'")
    );
    assert_eq!(
        reason(&target, json!("pytest"), "tests/test_a.py::TestGroup"),
        None
    );
}

#[test]
fn source_unreadable_file_is_one_parse_error_and_leaves_nodes_unchecked() {
    let target = Target::new();
    target.write("tests/test_bad.py", "x = 1\n\n'''never\nclosed\n");
    let findings = check(
        &target,
        json!("pytest"),
        &["tests/test_bad.py::test_a", "tests/test_bad.py::test_b"],
    );
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].code, "PARSE_ERROR");
    assert_eq!(findings[0].severity, Severity::Warning);
    assert_eq!(
        findings[0].provenance,
        Provenance::new("tests/test_bad.py", 3)
    );
    assert_eq!(
        findings[0].message,
        "could not read python source 'tests/test_bad.py': unterminated string"
    );
}

#[test]
fn source_invalid_utf8_reports_its_line() {
    let target = Target::new();
    let path = target.path().join("t.rs");
    std::fs::write(&path, b"fn a() {}\nfn \xff() {}\n").unwrap();
    let findings = check(&target, json!("cargo"), &["t.rs::a"]);
    assert_eq!(
        messages(&findings),
        ["could not read rust source 't.rs': invalid UTF-8"]
    );
    assert_eq!(findings[0].provenance.line().map(u32::from), Some(2));
}

// --- rust ---------------------------------------------------------------

fn rust_scope(text: &str) -> Scope {
    rust::scan(text).unwrap_or_else(|error| panic!("{error:?}"))
}

fn names(scope: &Scope) -> Vec<&str> {
    scope.names.iter().map(String::as_str).collect()
}

#[test]
fn source_rust_top_level_and_inline_modules() {
    let scope = rust_scope(
        "\u{feff}use x;\r\n#[test]\r\nfn loads_profile() {}\r\n\
         #[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn splits() {}\n\
             pub(crate) mod deep { #[test] async fn inner() {} }\n}\nmod out_of_line;\n",
    );
    assert_eq!(names(&scope), ["loads_profile"]);
    assert_eq!(names(&scope.children["tests"]), ["splits"]);
    assert_eq!(names(&scope.children["tests"].children["deep"]), ["inner"]);
    assert!(!scope.children.contains_key("out_of_line"));
}

#[test]
fn source_rust_qualifiers_and_raw_identifiers() {
    let scope = rust_scope(
        "pub(crate) async unsafe extern \"C\" fn qualified() {}\n\
         const fn konst() {}\nfn r#match() {}\nfn generic<'a, T: Fn() -> u8>(x: &'a T) {}\n",
    );
    assert_eq!(names(&scope), ["generic", "konst", "match", "qualified"]);
}

#[test]
fn source_rust_fn_in_impl_body_macro_or_extern_block_does_not_count() {
    let scope = rust_scope(
        "impl S { fn ghost(&self) {} }\ntrait T { fn ghost(); }\n\
         fn outer() { fn ghost() {} }\nmacro_rules! m { () => { fn ghost() {} } }\n\
         m! { fn ghost() {} }\nextern \"C\" { fn ghost(); }\ntype F = fn(u8);\n",
    );
    assert_eq!(names(&scope), ["outer"]);
}

#[test]
fn source_rust_fn_in_strings_and_comments_does_not_count() {
    let scope = rust_scope(
        "const A: &str = r##\"fn ghost() {} \"# \"##;\nconst B: &[u8] = br#\"fn ghost\"#;\n\
         const C: &str = \"fn ghost() { \\\" \";\nconst D: &[u8] = b\"fn ghost\";\n\
         /* outer /* fn ghost() {} */ still comment fn ghost */\n\
         /// fn ghost() {}\n//! fn ghost\n// fn ghost() {\nconst E: char = '{';\n\
         const G: u8 = b'}';\nconst H: char = '\\'';\nfn real() {}\n",
    );
    assert_eq!(names(&scope), ["real"]);
}

#[test]
fn source_rust_lifetime_is_not_a_char_literal() {
    let scope =
        rust_scope("fn helper<'a>(x: &'a str) -> &'static str { 'outer: loop {} }\nfn real() {}\n");
    assert_eq!(names(&scope), ["helper", "real"]);
}

#[test]
fn source_rust_attribute_with_braces_does_not_hide_items() {
    let scope = rust_scope("#[doc = \"{\"]\n#[cfg_attr(x, path = \"a}\")]\nfn real() {}\n");
    assert_eq!(names(&scope), ["real"]);
}

#[test]
fn source_rust_failures_carry_their_line() {
    let cases = [
        (
            "fn a() {}\n\n\n\n/* /* */\nfn b() {}\n",
            5,
            "unterminated block comment",
        ),
        (
            "fn a() {}\nconst S: &str = r#\"open\n",
            2,
            "unterminated raw string",
        ),
        ("const S: &str = \"open\n", 1, "unterminated string"),
        ("fn a() {\n", 1, "unbalanced bracket '{'"),
        ("fn a() {}\n}\n", 2, "unbalanced bracket '}'"),
    ];
    for (text, line, reason) in cases {
        assert_eq!(
            rust::scan(text).unwrap_err(),
            ScanError::new(line, reason),
            "{text:?}"
        );
    }
}

// --- python -------------------------------------------------------------

fn python_scope(text: &str) -> Scope {
    python::scan(text).unwrap_or_else(|error| panic!("{error:?}"))
}

#[test]
fn source_python_top_level_and_direct_methods() {
    let scope = python_scope(concat!(
        "\u{feff}import x\r\n\r\n@pytest.mark.slow\r\ndef test_a():\r\n    pass\r\n",
        "async def test_b(): pass\n",
        "class TestGroup(Base):  # noqa: header comment\n",
        "    \"\"\"Doc.\"\"\"\n",
        "    @decorated\n",
        "    async def test_c(self):\n",
        "        def inner(): pass\n",
        "    class Nested:\n",
        "        def test_nested(self): pass\n",
        "    def test_d(self): pass\n",
        "class Single: pass\n",
    ));
    assert_eq!(names(&scope), ["Single", "TestGroup", "test_a", "test_b"]);
    assert_eq!(names(&scope.children["TestGroup"]), ["test_c", "test_d"]);
    assert!(scope.children["Single"].names.is_empty());
}

#[test]
fn source_python_strings_and_conditionals_do_not_count() {
    let scope = python_scope(
        "'''\ndef test_ghost(): pass\n'''\nR = rb'def test_ghost'  # def test_ghost\n\
         if True:\n    def test_cond(): pass\ntry:\n    def test_try(): pass\nexcept E:\n    pass\n\
         def real(): pass\n",
    );
    assert_eq!(names(&scope), ["real"]);
}

#[test]
fn source_python_bracket_and_backslash_continuation() {
    let scope = python_scope(
        "class TestGroup:\n    def test_a(self):\n        call(1,\n2,\ndef_=3)\n    def test_after(self): pass\n\
         x = 1 + \\\ndef_not = 2\n\tdef test_tab(self): pass\n",
    );
    assert_eq!(
        names(&scope.children["TestGroup"]),
        ["test_a", "test_after"]
    );
    assert_eq!(names(&scope), ["TestGroup"]);
}

#[test]
fn source_python_tab_indented_class_body() {
    let scope = python_scope("class T:\n\tdef test_a(self): pass\n\tdef test_b(self): pass\n");
    assert_eq!(names(&scope.children["T"]), ["test_a", "test_b"]);
}

#[test]
fn source_python_pep701_fstrings() {
    let scope = python_scope(
        "x = f\"{\"def test_fake(): pass\"}\"\ny = f'{a:{width}.{p}} {{literal}} {d[\"k\"]!r}'\n\
         z = rf'''{\n\"\"\"def test_fake2()\"\"\"\n}'''\ndef test_real(): pass\n",
    );
    assert_eq!(names(&scope), ["test_real"]);
}

#[test]
fn source_python_fstring_field_comments_are_comments() {
    // A comment in a multi-line replacement field runs to the line's end, so
    // a quote or bracket inside it is not code (PEP 701).
    for text in [
        "value = f\"{1 # unmatched quote '\n}\"\ndef test_real():\n    pass\n",
        "x = f\"{1 # unmatched quote \"\n}\"\ndef test_real(): pass\n",
        "v = f\"{(1 # } ) \"\n)}\"\ndef test_real(): pass\n",
    ] {
        assert_eq!(names(&python_scope(text)), ["test_real"], "{text:?}");
    }
}

#[test]
fn source_python_failures_carry_their_line() {
    let cases = [
        ("x = 1\n\n'''never\n", 3, "unterminated string"),
        ("x = 'open\ny = 2\n", 1, "unterminated string"),
        ("x = (1,\n2\n", 1, "unbalanced bracket '('"),
        ("x = 1)\n", 1, "unbalanced bracket ')'"),
    ];
    for (text, line, reason) in cases {
        assert_eq!(
            python::scan(text).unwrap_err(),
            ScanError::new(line, reason),
            "{text:?}"
        );
    }
}

// --- go -----------------------------------------------------------------

fn go_scope(text: &str) -> Scope {
    go::scan(text).unwrap_or_else(|error| panic!("{error:?}"))
}

#[test]
fn source_go_top_level_funcs_only() {
    let scope = go_scope(
        "\u{feff}package store\r\n\r\nimport \"testing\"\r\n\r\nfunc TestPut(t *testing.T) {\r\n\
         \tt.Run(\"TestGhost\", func(t *testing.T) {})\n}\n\
         func TestG[T any](t *testing.T) {}\nfunc (s *Suite) TestGhost() {}\n\
         var f = func() {}\ntype F func(int)\n// func TestGhost() {}\n\
         /* func TestGhost() {} */\nvar s = `func TestGhost() {\n}`\nvar r = '\\''\n\
         var q = \"func TestGhost() {\"\nfunc /* c */ TestAfterComment() {}\n",
    );
    assert_eq!(names(&scope), ["TestAfterComment", "TestG", "TestPut"]);
}

#[test]
fn source_go_failures_carry_their_line() {
    let cases = [
        (
            "package p\n\n\n\n\n\n\n\nvar s = `open\n",
            9,
            "unterminated raw string",
        ),
        ("package p\n/* open\n", 2, "unterminated block comment"),
        ("package p\nfunc A() {\n", 2, "unbalanced bracket '{'"),
        ("package p\nvar s = \"open\n", 2, "unterminated string"),
        ("package p\nvar r = '\\\n", 2, "unterminated rune literal"),
    ];
    for (text, line, reason) in cases {
        assert_eq!(
            go::scan(text).unwrap_err(),
            ScanError::new(line, reason),
            "{text:?}"
        );
    }
}

#[test]
fn source_go_block_comments_do_not_nest() {
    // The first `*/` closes the comment, so `func Real` after it is code.
    let scope = go_scope("package p\n/* /* */ func Real() {}\n");
    assert_eq!(names(&scope), ["Real"]);
}

#[test]
fn line_fits_scans_a_file_of_the_largest_line_count() {
    assert_eq!(beyond_lines(4_294_967_295), None);
}

#[test]
fn line_fits_refuses_a_file_with_one_line_more() {
    let refusal = beyond_lines(4_294_967_296).expect("refused");
    assert!(refusal.contains("4294967296"), "{refusal}");
}
