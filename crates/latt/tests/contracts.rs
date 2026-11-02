//! The contract artifacts under `contracts/`: the CLI's commands and flags, the
//! finding codes, and the shipped profiles' ID patterns.
//!
//! Two checks. *Current*: each artifact matches what this build generates
//! (regenerate with `LATTICE_REGEN_BASELINES=1`; the diff is the review).
//! *Released*: each artifact is compared with the copy frozen at the last
//! release, `contracts/released/`. A removal, a rename, a flag turned required,
//! a changed default, a dropped possible value, a removed code, a changed
//! default severity or a changed ID pattern is a break. It must be listed under
//! Breaking in Unreleased or the workspace version's section, read by
//! `tools/check-breaking.sh`, and the pending release must be a minor one. On a patch line every break fails. Additions
//! pass.
//!
//! `cli.txt` is read off `--help` alone. Exit classes and stream routing are
//! not in it; `tests/cli.rs` pins those.
// Requirement: versioning/Pre-1.0 semver policy

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

#[path = "common/regen.rs"]
mod regen;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The CLI artifact for this feature set: `browse` exists only under `tui`.
fn cli_file() -> &'static str {
    if cfg!(feature = "tui") {
        "cli-tui.txt"
    } else {
        "cli.txt"
    }
}

// ---- generators -------------------------------------------------------------

/// `<binary> <args> -h`, the one-line-per-flag help, with color and terminal width out of the picture.
fn help(binary: &Path, args: &[String]) -> String {
    let output = Command::new(binary)
        .args(args)
        .arg("-h")
        .env("NO_COLOR", "1")
        .env_remove("FORCE_COLOR")
        .env("TERM", "dumb")
        .output()
        .unwrap();
    assert!(output.status.success(), "{args:?} -h failed");
    String::from_utf8(output.stdout).unwrap()
}

/// The lines of a help section such as `Options:`, up to the next blank line.
fn section<'a>(text: &'a str, title: &str) -> Vec<&'a str> {
    text.lines()
        .skip_while(|l| *l != title)
        .skip(1)
        .take_while(|l| !l.is_empty())
        .collect()
}

/// One row per command, option and positional argument, found by walking
/// `--help` from the top. Option rows: path, flag, value name, required or
/// optional, default, possible values. Help text itself is not part of it.
fn cli_contract(binary: &Path) -> String {
    let value = Regex::new(r"^(?:-\w, )?(--[\w-]+)(?: <([^>]+)>)?").unwrap();
    let default = Regex::new(r"\[default: ([^\]]*)\]").unwrap();
    let possible = Regex::new(r"\[possible values: ([^\]]*)\]").unwrap();
    let bracketed = Regex::new(r"\[[^\]]*\]").unwrap();
    let mut rows = Vec::new();
    let mut pending = vec![Vec::<String>::new()];
    while let Some(path) = pending.pop() {
        let text = help(binary, &path);
        let name = if path.is_empty() {
            "latt".to_string()
        } else {
            format!("latt {}", path.join(" "))
        };
        rows.push(format!("command\t{name}"));
        let usage = text
            .lines()
            .find_map(|l| l.strip_prefix("Usage: "))
            .unwrap_or_default();
        // A required item is one the usage line names outside brackets.
        let required: BTreeSet<String> = bracketed
            .replace_all(usage, "")
            .split_whitespace()
            .map(str::to_string)
            .collect();
        for line in section(&text, "Options:") {
            let line = line.trim_start();
            let Some(caps) = value.captures(line) else {
                continue;
            };
            let flag = &caps[1];
            if flag == "--help" {
                continue;
            }
            let field = |re: &Regex| {
                re.captures(line)
                    .map_or_else(|| "-".into(), |c| c[1].to_string())
            };
            rows.push(format!(
                "option\t{name}\t{flag}\t{}\t{}\t{}\t{}",
                caps.get(2).map_or("-", |m| m.as_str()),
                if required.contains(flag) {
                    "required"
                } else {
                    "optional"
                },
                field(&default),
                field(&possible),
            ));
        }
        for line in section(&text, "Arguments:") {
            let arg = line.split_whitespace().next().unwrap_or_default();
            let bare = arg.trim_matches(['<', '>', '[', ']', '.']);
            let needed = required
                .iter()
                .any(|t| t.trim_end_matches("...") == format!("<{bare}>"));
            rows.push(format!(
                "argument\t{name}\t{bare}\t{}",
                if needed { "required" } else { "optional" }
            ));
        }
        for line in section(&text, "Commands:").iter().rev() {
            let command = line.split_whitespace().next().unwrap_or_default();
            if command != "help" {
                let mut next = path.clone();
                next.push(command.to_string());
                pending.push(next);
            }
        }
    }
    rows.sort();
    rows.join("\n") + "\n"
}

/// Codes a Python adapter emits. They cannot be read from Rust, so this list is
/// kept by hand, and `every_code_literal_is_registered` checks it.
/// `code`, owner, default severity, whether every finding carries a node. An
/// adapter code has no default in the core, so its severity column is `-`, and
/// the core cannot promise a node for it, so its node column is `no`. The node
/// column is last so the severity column keeps its index against `released/`.
fn codes_contract() -> String {
    let mut rows = BTreeMap::new();
    for (code, owner, severity, node) in latt_core::validate::core_codes() {
        rows.insert(
            code.to_string(),
            format!("{owner}\t{}\t{}", severity.as_str(), yes_no(node)),
        );
    }
    for code in latt_core::validate::shipped_codes() {
        rows.entry(code.to_string())
            .or_insert_with(|| "adapter\t-\tno".into());
    }
    rows.iter()
        .map(|(code, rest)| format!("{code}\t{rest}\n"))
        .collect()
}

fn yes_no(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

/// `code`, replacement, release retired, for every code a profile may no
/// longer name.
fn retired_codes_contract() -> String {
    latt_core::validate::RETIRED_CODES
        .iter()
        .map(|(code, replacement, release)| format!("{code}\t{replacement}\t{release}\n"))
        .collect()
}

/// Profile file, kind, `id_pattern` (`-` for none), for every profile under
/// `profiles`, read through `show` so inherited kinds count.
fn id_patterns_contract(binary: &Path, profiles: &Path) -> String {
    let mut files: Vec<PathBuf> = fs::read_dir(profiles)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    files.sort();
    let mut rows = String::new();
    for file in files {
        let output = Command::new(binary)
            .args(["show", "--profile"])
            .arg(&file)
            .output()
            .unwrap();
        assert!(output.status.success(), "resolve {}", file.display());
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let name = file.file_name().unwrap().to_string_lossy();
        // Sorted here, not by `serde_json`: a pinned artifact must not depend
        // on whether `preserve_order` is unified into the build.
        let kinds: BTreeMap<&String, &serde_json::Value> =
            document["node_kinds"].as_object().unwrap().iter().collect();
        for (kind, spec) in kinds {
            let pattern = spec["id_pattern"].as_str().unwrap_or("-");
            rows.push_str(&format!("{name}\t{kind}\t{pattern}\n"));
        }
    }
    rows
}

fn current_artifacts() -> Vec<(&'static str, String)> {
    let binary = Path::new(env!("CARGO_BIN_EXE_latt"));
    vec![
        (cli_file(), cli_contract(binary)),
        ("codes.tsv", codes_contract()),
        ("retired-codes.tsv", retired_codes_contract()),
        (
            "id-patterns.tsv",
            id_patterns_contract(binary, &root().join("profiles")),
        ),
    ]
}

// ---- current ----------------------------------------------------------------

#[test]
fn contract_artifacts_match_the_build() {
    let dir = root().join("contracts");
    for (file, generated) in current_artifacts() {
        let path = dir.join(file);
        if regen::regen_baselines() {
            fs::write(&path, &generated).unwrap();
            continue;
        }
        let pinned = fs::read_to_string(&path).unwrap_or_default();
        assert_eq!(
            pinned, generated,
            "{file} differs from the build; regenerate with LATTICE_REGEN_BASELINES=1 and review the diff"
        );
    }
}

/// Environment variables and other code-shaped literals that are not codes.
const NOT_CODES: &[&str] = &[
    "CARGO_MANIFEST_DIR",
    "CARGO_PKG_VERSION",
    "FORCE_COLOR",
    "LC_ALL",
    "LC_CTYPE",
    "NO_COLOR",
];

/// Every code-shaped string literal in the library and adapter sources is a
/// registered code, so a code emitted by a literal cannot miss `codes.tsv`.
#[test]
fn every_code_literal_is_registered() {
    // A retired code is a literal too: the loader names it to refuse it.
    let registered: BTreeSet<String> = codes_contract()
        .lines()
        .chain(retired_codes_contract().lines())
        .map(|l| l.split('\t').next().unwrap().to_string())
        .collect();
    let literal = Regex::new(r#""([A-Z]+(?:_[A-Z]+)+)""#).unwrap();
    let mut files = Vec::new();
    for entry in fs::read_dir(root().join("crates")).unwrap() {
        let dir = entry.unwrap().path();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        if name == "latt-core" || name.starts_with("latt-adapter-") {
            collect(&dir.join("src"), "rs", &mut files);
        }
    }
    collect(&root().join("adapters"), "py", &mut files);
    let mut unknown = BTreeSet::new();
    for file in files {
        for caps in literal.captures_iter(&fs::read_to_string(&file).unwrap()) {
            let code = &caps[1];
            if !registered.contains(code) && !NOT_CODES.contains(&code) {
                unknown.insert(format!("{code} in {}", file.display()));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "unregistered code literals: {unknown:#?}"
    );
}

fn collect(dir: &Path, extension: &str, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "__pycache__") {
                collect(&path, extension, files);
            }
        } else if path.extension().is_some_and(|e| e == extension) {
            files.push(path);
        }
    }
}

// ---- released ---------------------------------------------------------------

/// Rows keyed by their identifying columns: what a row is, apart from what it
/// says about it.
fn keyed(text: &str, key_columns: usize) -> BTreeMap<String, Vec<String>> {
    text.lines()
        .map(|line| {
            let columns: Vec<String> = line.split('\t').map(str::to_string).collect();
            let split = key_columns.min(columns.len());
            (columns[..split].join(" "), columns[split..].to_vec())
        })
        .collect()
}

/// `(lint, item)` for every break from `released` to `current` in one artifact.
fn breaks(file: &str, released: &str, current: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut add = |lint: &str, item: String| found.push((lint.to_string(), item));
    if file.starts_with("cli") {
        // Commands and positionals have no third key column; options do.
        let (released, current) = (keyed(released, 3), keyed(current, 3));
        // An input added to a command the release already had, and required:
        // every released invocation of that command now fails. The same input
        // under a new command breaks nothing.
        for (key, new) in &current {
            // `<kind> <command words...> <input>`: the input is the last word.
            let words: Vec<&str> = key.split(' ').collect();
            let kind = words[0];
            if kind == "command" || words.len() < 3 || released.contains_key(key) {
                continue;
            }
            let command = words[1..words.len() - 1].join(" ");
            let required_at = usize::from(kind == "option");
            if released.contains_key(&format!("command {command}"))
                && new.get(required_at).is_some_and(|r| r == "required")
            {
                add("cli_now_required", words[1..].join(" "));
            }
        }
        for (key, old) in &released {
            let mut words = key.splitn(2, ' ');
            let kind = words.next().unwrap_or_default();
            let item = words.next().unwrap_or_default().to_string();
            let Some(new) = current.get(key) else {
                let lint = match kind {
                    "command" => "cli_command_removed",
                    "option" => "cli_option_removed",
                    _ => "cli_argument_removed",
                };
                add(lint, item);
                continue;
            };
            let column = |row: &[String], i: usize| row.get(i).cloned().unwrap_or_default();
            // Option columns after the key: value name, requiredness, default,
            // possible values. Argument columns: requiredness.
            let required_at = usize::from(kind == "option");
            if column(old, required_at) == "optional" && column(new, required_at) == "required" {
                add("cli_now_required", item.clone());
            }
            if kind == "option" {
                if column(old, 2) != column(new, 2) {
                    add("cli_default_changed", item.clone());
                }
                let values = |row: &[String]| -> BTreeSet<String> {
                    column(row, 3).split(", ").map(str::to_string).collect()
                };
                if column(old, 3) != "-" && !values(old).is_subset(&values(new)) {
                    add("cli_value_removed", item);
                }
            }
        }
    } else if file == "codes.tsv" {
        let (released, current) = (keyed(released, 1), keyed(current, 1));
        for (code, old) in &released {
            match current.get(code) {
                None => add("code_removed", code.clone()),
                Some(new) if new.get(1) != old.get(1) => {
                    add("code_severity_changed", code.clone());
                }
                Some(_) => {}
            }
        }
    } else if file == "retired-codes.tsv" {
        // Un-retiring a code lets a profile that the release refused load
        // again, in silence: a contract change like any other.
        let (released, current) = (keyed(released, 1), keyed(current, 1));
        for code in released.keys() {
            if !current.contains_key(code) {
                add("retired_code_unretired", code.clone());
            }
        }
    } else {
        let (released, current) = (keyed(released, 2), keyed(current, 2));
        for (kind, old) in &released {
            match current.get(kind) {
                None => add("id_kind_removed", kind.clone()),
                Some(new) if new != old => add("id_pattern_changed", kind.clone()),
                Some(_) => {}
            }
        }
    }
    found
}

/// `(major, minor, patch, pre-release)` of a version string.
fn semver(version: &str) -> (u64, u64, u64, Option<String>) {
    let (release, pre) = match version.trim().split_once('-') {
        Some((release, pre)) => (release, Some(pre.to_string())),
        None => (version.trim(), None),
    };
    let mut parts = release.split('.').map(|p| p.parse::<u64>().unwrap());
    let mut next = || parts.next().unwrap();
    (next(), next(), next(), pre)
}

/// True when the release `current` is heading for is a minor (or major) one
/// relative to `released`. A pre-release of the same version is still on its
/// way to that version: from `0.8.0-rc.8` to `0.8.0-rc.9` the pending release
/// is `0.8.0`, a minor bump over 0.7, not a patch on the 0.8 line.
fn pending_release_is_minor(released: &str, current: &str) -> bool {
    let (r_major, r_minor, r_patch, r_pre) = semver(released);
    let (c_major, c_minor, c_patch, _) = semver(current);
    (c_major, c_minor) > (r_major, r_minor)
        || (r_pre.is_some() && (c_major, c_minor, c_patch) == (r_major, r_minor, r_patch))
}

/// The comparator over hand-written rows: what counts as a break, and what
/// an addition is allowed to be.
#[test]
fn cli_breaks_are_classified() {
    let released = "command\tlatt validate\noption\tlatt validate\t--profile\t<PROFILE>\trequired\t-\t-\noption\tlatt validate\t--adapter\t<ADAPTER>\toptional\t-\t-\n";
    let case = |current: &str| breaks("cli.txt", released, current);
    assert_eq!(case(released), []);
    // A required input appended to an existing command breaks its callers,
    // whether option or positional; an optional one does not.
    assert_eq!(
        case(&format!(
            "{released}option\tlatt validate\t--tenant\t<TENANT>\trequired\t-\t-\n"
        )),
        [(
            "cli_now_required".to_string(),
            "latt validate --tenant".to_string()
        )]
    );
    assert_eq!(
        case(&format!(
            "{released}argument\tlatt validate\tREGISTER\trequired\n"
        )),
        [(
            "cli_now_required".to_string(),
            "latt validate REGISTER".to_string()
        )]
    );
    assert_eq!(
        case(&format!(
            "{released}option\tlatt validate\t--tenant\t<TENANT>\toptional\t-\t-\n"
        )),
        []
    );
    // The same required input under a new command is an addition.
    assert_eq!(
        case(&format!(
            "{released}command\tlatt audit\nargument\tlatt audit\tREGISTER\trequired\n"
        )),
        []
    );
    // An existing optional input turned required, and a removal.
    assert_eq!(
        case(&released.replace(
            "--adapter\t<ADAPTER>\toptional",
            "--adapter\t<ADAPTER>\trequired"
        )),
        [(
            "cli_now_required".to_string(),
            "latt validate --adapter".to_string()
        )]
    );
    assert_eq!(
        case(
            "command\tlatt validate\noption\tlatt validate\t--profile\t<PROFILE>\trequired\t-\t-\n"
        ),
        [(
            "cli_option_removed".to_string(),
            "latt validate --adapter".to_string()
        )]
    );
}

#[test]
fn pending_release_rule() {
    assert!(pending_release_is_minor("0.8.0-rc.8", "0.8.0-rc.9"));
    assert!(pending_release_is_minor("0.8.0-rc.8", "0.8.0"));
    assert!(pending_release_is_minor("0.8.0", "0.9.0-rc.1"));
    assert!(!pending_release_is_minor("0.8.0", "0.8.1"));
    assert!(!pending_release_is_minor("0.8.1", "0.8.2-rc.1"));
}

/// Every break since the last release is listed under Breaking, on a minor
/// release; on a patch line there are none.
#[test]
fn contract_breaks_since_the_release_are_listed() {
    let dir = root().join("contracts/released");
    let released_version = fs::read_to_string(dir.join("VERSION")).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    let mut log = String::new();
    let mut all = Vec::new();
    for (file, current) in current_artifacts() {
        // An artifact the release predates has nothing to break from.
        let released = fs::read_to_string(dir.join(file)).unwrap_or_default();
        let found = breaks(file, &released, &current);
        log.push_str(&format!("== semver-checks: contracts {file}\n"));
        for (lint, item) in &found {
            log.push_str(&format!(
                "--- failure {lint}: contract break ---\nFailed in:\n  {item}\n"
            ));
        }
        all.extend(found);
    }
    if all.is_empty() {
        return;
    }
    assert!(
        pending_release_is_minor(&released_version, version),
        "{version} is a patch release after {}, which may not break a contract: {all:?}",
        released_version.trim()
    );
    let log_path =
        std::env::temp_dir().join(format!("lattice-contracts-{}.log", std::process::id()));
    fs::write(&log_path, log).unwrap();
    let output = Command::new("sh")
        .arg(root().join("tools/check-breaking.sh"))
        .arg(root().join("CHANGELOG.md"))
        .arg(version)
        .arg(&log_path)
        .output()
        .unwrap();
    let _ = fs::remove_file(&log_path);
    assert!(
        output.status.success(),
        "contract breaks since {} are not all listed under Breaking:\n{}{}",
        released_version.trim(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Write `contracts/released/` from a release's binary and profiles:
/// `LATTICE_SEED_BINARY` and `LATTICE_SEED_PROFILES`. Run once per release
/// cut, per feature set, from the release commit itself: `codes.tsv` and
/// `VERSION` come from this tree, so a binary of another version is refused.
/// A release that predates the generator needs them written by hand.
#[test]
#[ignore = "run by hand at a release cut"]
fn seed_released_contracts() {
    let binary = PathBuf::from(std::env::var("LATTICE_SEED_BINARY").unwrap());
    let profiles = PathBuf::from(std::env::var("LATTICE_SEED_PROFILES").unwrap());
    let version = env!("CARGO_PKG_VERSION");
    let reported = Command::new(&binary).arg("--version").output().unwrap();
    let reported = String::from_utf8_lossy(&reported.stdout);
    assert_eq!(
        reported.trim().rsplit(' ').next(),
        Some(version),
        "seed binary reports '{}', not this tree's {version}",
        reported.trim()
    );
    let dir = root().join("contracts/released");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(cli_file()), cli_contract(&binary)).unwrap();
    fs::write(
        dir.join("id-patterns.tsv"),
        id_patterns_contract(&binary, &profiles),
    )
    .unwrap();
    fs::write(dir.join("codes.tsv"), codes_contract()).unwrap();
    fs::write(dir.join("retired-codes.tsv"), retired_codes_contract()).unwrap();
    fs::write(dir.join("VERSION"), format!("{version}\n")).unwrap();
}
