//! The built-in adapters reach the graph through ingest alone: within `latt-core`,
//! only the native dispatch in `src/document.rs` may call into `adapters`.
//!
//! The module is `pub` because a host may run one adapter directly, so visibility
//! cannot hold the line; this scan does.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, recursively.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("the source tree is readable") {
        let path = entry.expect("the source tree is readable").path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// `file:line` for every line outside the dispatch and the adapters that names
/// the `adapters` module.
fn violations(src: &Path) -> Vec<String> {
    let mut files = Vec::new();
    sources(src, &mut files);
    let mut found = Vec::new();
    for file in files {
        let relative = file.strip_prefix(src).unwrap();
        if relative == Path::new("document.rs") || relative.starts_with("adapters") {
            continue;
        }
        let text = fs::read_to_string(&file).unwrap();
        for (number, line) in text.lines().enumerate() {
            if line.contains("adapters::") {
                found.push(format!("src/{}:{}", relative.display(), number + 1));
            }
        }
    }
    found.sort();
    found
}

// Requirement: builtin-adapter-dispatch/Native execution produces the same document
#[test]
fn only_the_dispatch_calls_the_built_in_adapters() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert_eq!(violations(&src), Vec::<String>::new());
}

// Requirement: builtin-adapter-dispatch/Native execution produces the same document
#[test]
fn a_direct_call_is_named_by_file_and_line() {
    let dir = std::env::temp_dir().join(format!("latt_adapters_boundary_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("adapters")).unwrap();
    fs::write(dir.join("document.rs"), "crate::adapters::md::run();\n").unwrap();
    fs::write(dir.join("adapters/md.rs"), "super::adapters::toml::x();\n").unwrap();
    fs::write(
        dir.join("trace.rs"),
        "fn f() {}\nlet d = crate::adapters::md::build();\n",
    )
    .unwrap();

    let found = violations(&dir);
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(found, ["src/trace.rs:2"]);
}
