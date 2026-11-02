//! Turning a node kind's declared owner action into an argv browse can run.
//!
//! Pure functions of the action, the node and the environment they are
//! handed, so every refusal is testable without a terminal or a process.

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitStatus, Stdio};

use latt_core::graph::Node;
use latt_core::profile::{ActionDecl, Segment};

use super::app::OwnerRun;
use serde_json::Value;

/// Why an action cannot be launched; its `Display` is the status-line text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The node has no value (absent or null) for an `{attrs.…}` placeholder.
    MissingAttr { verb: String, attr: String },
    /// The attr's value is a list or a map, which is not one argument.
    NotScalar { verb: String, attr: String },
    /// `{provenance.line}` was asked for and the line is unknown.
    UnknownLine { verb: String },
    /// An argument would start with `-` from register text.
    LooksLikeOption { verb: String, arg: String },
    /// The program is not on `PATH` or not at the path it names.
    ProgramNotFound { program: String },
    /// The profile uses a placeholder this build of browse does not know.
    UnknownPlaceholder { verb: String },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAttr { verb, attr } => {
                write!(f, "cannot run {verb}: the node has no '{attr}'")
            }
            Self::NotScalar { verb, attr } => {
                write!(f, "cannot run {verb}: '{attr}' is not a single value")
            }
            Self::UnknownLine { verb } => {
                write!(f, "cannot run {verb}: the node's source line is unknown")
            }
            Self::LooksLikeOption { verb, arg } => write!(
                f,
                "cannot run {verb}: '{arg}' would be read as an option (put \"--\" before it in the profile)"
            ),
            Self::ProgramNotFound { program } => write!(f, "{program} was not found"),
            Self::UnknownPlaceholder { verb } => {
                write!(
                    f,
                    "cannot run {verb}: it uses a placeholder this browse does not know"
                )
            }
        }
    }
}

/// Build the argv for `decl` against `node`.
///
/// Each template stays one argument. An argument that starts with a
/// placeholder and resolves to `-…` is refused unless an earlier argument is
/// a literal `--`: register text must not become an option.
pub fn resolve(verb: &str, decl: &ActionDecl, node: &Node) -> Result<Vec<String>, Refusal> {
    let mut argv = Vec::with_capacity(decl.run.len());
    let mut options_ended = false;
    for template in &decl.run {
        let mut arg = String::new();
        for segment in &template.segments {
            match segment {
                Segment::Literal(text) => arg.push_str(text),
                Segment::Id => arg.push_str(&node.id),
                Segment::Kind => arg.push_str(&node.kind),
                Segment::ProvenanceFile => arg.push_str(node.provenance.file()),
                Segment::ProvenanceLine => {
                    let Some(line) = node.provenance.line() else {
                        return Err(Refusal::UnknownLine {
                            verb: verb.to_owned(),
                        });
                    };
                    arg.push_str(&line.to_string());
                }
                Segment::Attr(attr) => match node.attrs.get(attr) {
                    None | Some(Value::Null) => {
                        return Err(Refusal::MissingAttr {
                            verb: verb.to_owned(),
                            attr: attr.clone(),
                        });
                    }
                    Some(Value::String(text)) => arg.push_str(text),
                    Some(Value::Number(n)) => arg.push_str(&n.to_string()),
                    Some(Value::Bool(b)) => arg.push_str(if *b { "true" } else { "false" }),
                    Some(Value::Array(_) | Value::Object(_)) => {
                        return Err(Refusal::NotScalar {
                            verb: verb.to_owned(),
                            attr: attr.clone(),
                        });
                    }
                },
                // A newer core can parse a placeholder this binary predates.
                _ => {
                    return Err(Refusal::UnknownPlaceholder {
                        verb: verb.to_owned(),
                    });
                }
            }
        }
        let starts_with_value = !matches!(template.segments.first(), Some(Segment::Literal(_)));
        if starts_with_value && !options_ended && arg.starts_with('-') {
            return Err(Refusal::LooksLikeOption {
                verb: verb.to_owned(),
                arg,
            });
        }
        if template.source == "--" {
            options_ended = true;
        }
        argv.push(arg);
    }
    Ok(argv)
}

/// Find the program an argv names: a bare name on `path`, a name with `/`
/// against `target_dir`. `None` when there is no executable file there.
pub fn find_program(
    program: &str,
    path: Option<&OsStr>,
    target_dir: Option<&Path>,
) -> Option<PathBuf> {
    if program.contains('/') {
        let candidate = match target_dir {
            Some(dir) if Path::new(program).is_relative() => dir.join(program),
            _ => PathBuf::from(program),
        };
        return is_executable(&candidate).then_some(candidate);
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// The owner program as a command: its argv, in the target directory.
pub fn owner_command(run: &OwnerRun) -> ProcessCommand {
    let mut cmd = ProcessCommand::new(&run.program);
    cmd.args(&run.argv[1..]);
    if let Some(dir) = &run.cwd {
        cmd.current_dir(dir);
    }
    cmd
}

/// How an exit reads on the status line and the output pane.
pub fn describe_exit(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exited {code}"),
        None => "ended by a signal".to_owned(),
    }
}

/// The most a capture keeps of each stream.
pub const CAPTURE_LIMIT: u64 = 1 << 20;

/// Read up to [`CAPTURE_LIMIT`] of a stream, then drain the rest so the
/// program never blocks on a full pipe. True when output was cut.
pub fn read_capped(stream: impl Read + Send + 'static) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let mut stream = stream;
        let _ = (&mut stream).take(CAPTURE_LIMIT).read_to_end(&mut kept);
        let cut = io::copy(&mut stream, &mut io::sink()).is_ok_and(|n| n > 0);
        (kept, cut)
    })
}

/// Start a detached action with null stdio; a thread waits on it so it is
/// reaped without browse waiting.
pub fn run_detached(run: &OwnerRun) -> String {
    let mut cmd = owner_command(run);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            format!("{} started ({})", run.verb, run.argv[0])
        }
        Err(e) => format!("{} could not start: {e}", run.verb),
    }
}

#[cfg(test)]
fn node(id: &str, attrs: Value, line: u32) -> Node {
    let mut graph = latt_core::graph::LatticeGraph::new();
    graph
        .add_node(
            id,
            "issue",
            attrs.as_object().cloned().unwrap_or_default(),
            latt_core::types::Provenance::new("issues/a.toml", line),
        )
        .expect("one node");
    graph.node(id).cloned().expect("just added")
}

#[cfg(test)]
fn decl(run: &[&str]) -> ActionDecl {
    let yaml = format!(
        "name: t\nprofile_version: '1.0.0'\nedge_kinds: {{}}\nnode_kinds:\n  issue:\n    \
         id_pattern: '.*'\n    attrs: {{number: int, title: string, repo: string, open: bool, \
         labels: {{type: list, items: string}}}}\n    actions:\n      x: {{mode: capture, run: {}}}\n",
        serde_json::to_string(run).unwrap()
    );
    let profile =
        latt_core::profile::load_profile_value(serde_norway::from_str(&yaml).unwrap(), "t")
            .unwrap();
    profile.node_kinds()["issue"].actions[0].1.clone()
}

/// Requirement: browse-owner-actions/Argument resolution
#[cfg(test)]
mod resolve {
    use serde_json::json;

    use super::{Refusal, decl, node, resolve};

    #[test]
    fn placeholders_are_substituted_inside_longer_arguments() {
        let n = node("ISS-4", json!({"repo": "a/b", "number": 12}), 7);
        let argv = resolve(
            "web",
            &decl(&[
                "gh",
                "issue",
                "view",
                "--repo={attrs.repo}",
                "{attrs.number}",
            ]),
            &n,
        );
        assert_eq!(argv.unwrap(), ["gh", "issue", "view", "--repo=a/b", "12"]);
    }

    #[test]
    fn id_kind_and_provenance_are_substituted() {
        let n = node("ISS-4", json!({}), 7);
        let argv = resolve(
            "x",
            &decl(&["t", "{id}", "{kind}", "{provenance.file}:{provenance.line}"]),
            &n,
        );
        assert_eq!(argv.unwrap(), ["t", "ISS-4", "issue", "issues/a.toml:7"]);
    }

    #[test]
    fn a_bool_is_its_text_and_braces_are_literal() {
        let n = node("ISS-4", json!({"open": true}), 7);
        let argv = resolve("x", &decl(&["t", "{{{attrs.open}}}"]), &n);
        assert_eq!(argv.unwrap(), ["t", "{true}"]);
    }

    #[test]
    fn a_value_with_spaces_stays_one_argument() {
        let n = node("ISS-4", json!({"title": "fix the parser"}), 7);
        let argv = resolve("x", &decl(&["t", "{attrs.title}"]), &n);
        assert_eq!(argv.unwrap(), ["t", "fix the parser"]);
    }

    #[test]
    fn a_missing_or_null_attr_refuses_naming_it() {
        for attrs in [json!({}), json!({"number": null})] {
            let n = node("ISS-4", attrs, 7);
            let refusal = resolve("web", &decl(&["gh", "{attrs.number}"]), &n).unwrap_err();
            assert_eq!(
                refusal,
                Refusal::MissingAttr {
                    verb: "web".into(),
                    attr: "number".into()
                }
            );
            let text = refusal.to_string();
            assert!(text.contains("web") && text.contains("number"), "{text}");
        }
    }

    #[test]
    fn a_list_value_refuses() {
        // The profile declares `title` a string; a register can still emit a list.
        let n = node("ISS-4", json!({"title": ["a", "b"]}), 7);
        let refusal = resolve("x", &decl(&["t", "{attrs.title}"]), &n).unwrap_err();
        assert!(matches!(refusal, Refusal::NotScalar { .. }), "{refusal:?}");
    }

    #[test]
    fn an_unknown_line_refuses() {
        let n = node("ISS-4", json!({}), 0);
        let refusal = resolve("x", &decl(&["t", "{provenance.line}"]), &n).unwrap_err();
        assert_eq!(refusal, Refusal::UnknownLine { verb: "x".into() });
    }
}

/// Requirement: browse-owner-actions/Leading-dash guard
#[cfg(test)]
mod guard {
    use serde_json::json;

    use super::{Refusal, decl, node, resolve};

    #[test]
    fn an_id_that_looks_like_an_option_refuses() {
        let n = node("--exec=x", json!({}), 1);
        let refusal = resolve("show", &decl(&["ent", "show", "{id}"]), &n).unwrap_err();
        assert!(
            matches!(refusal, Refusal::LooksLikeOption { .. }),
            "{refusal:?}"
        );
        assert!(refusal.to_string().contains("option"), "{refusal}");
    }

    #[test]
    fn a_placeholder_starting_a_longer_argument_is_guarded() {
        let n = node("-rf", json!({}), 1);
        let refusal = resolve("x", &decl(&["t", "{id}.txt"]), &n).unwrap_err();
        assert!(
            matches!(refusal, Refusal::LooksLikeOption { .. }),
            "{refusal:?}"
        );
    }

    #[test]
    fn a_template_that_ends_options_first_runs() {
        let n = node("-1", json!({}), 1);
        let argv = resolve("show", &decl(&["ent", "show", "--", "{id}"]), &n);
        assert_eq!(argv.unwrap(), ["ent", "show", "--", "-1"]);
    }

    #[test]
    fn a_value_inside_a_literal_option_runs() {
        let n = node("ISS-1", json!({"repo": "-x"}), 1);
        let argv = resolve("x", &decl(&["gh", "--repo={attrs.repo}"]), &n);
        assert_eq!(argv.unwrap(), ["gh", "--repo=-x"]);
    }

    #[test]
    fn literal_template_options_are_never_refused() {
        let n = node("ISS-1", json!({}), 1);
        let argv = resolve("x", &decl(&["gh", "--web", "{id}"]), &n);
        assert_eq!(argv.unwrap(), ["gh", "--web", "ISS-1"]);
    }
}

/// Requirement: browse-owner-actions/Program lookup before launch
#[cfg(test)]
mod lookup {
    use std::ffi::OsString;
    use std::path::Path;

    use super::find_program;

    fn executable(dir: &Path, name: &str) {
        let file = dir.join(name);
        #[cfg(unix)]
        crate::stub::write_stub(&file, "#!/bin/sh\n");
        #[cfg(not(unix))]
        std::fs::write(&file, "#!/bin/sh\n").unwrap();
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lattice-owner-action-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_bare_name_is_found_on_path() {
        let a = scratch("path-a");
        let b = scratch("path-b");
        executable(&b, "ent");
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find_program("ent", Some(&path), None), Some(b.join("ent")));
    }

    #[test]
    fn a_bare_name_not_on_path_is_not_found() {
        let a = scratch("path-none");
        let path = OsString::from(a.as_os_str());
        assert_eq!(find_program("ent", Some(&path), None), None);
        assert_eq!(find_program("ent", None, None), None);
    }

    #[test]
    fn a_name_with_a_slash_resolves_against_the_target() {
        let target = scratch("target");
        std::fs::create_dir_all(target.join("bin")).unwrap();
        executable(&target.join("bin"), "tool");
        assert_eq!(
            find_program("./bin/tool", None, Some(&target)),
            Some(target.join("./bin/tool"))
        );
        assert_eq!(find_program("./bin/other", None, Some(&target)), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_is_not_executable_is_not_found() {
        let a = scratch("noexec");
        std::fs::write(a.join("ent"), "").unwrap();
        let path = OsString::from(a.as_os_str());
        assert_eq!(find_program("ent", Some(&path), None), None);
    }
}

#[cfg(test)]
fn run_of(argv: &[&str], cwd: &Path) -> OwnerRun {
    OwnerRun {
        verb: "x".to_owned(),
        argv: argv.iter().map(|a| (*a).to_owned()).collect(),
        program: PathBuf::from(argv[0]),
        mode: latt_core::profile::ActionMode::Capture,
        cwd: Some(cwd.display().to_string()),
    }
}

/// Requirement: browse-owner-actions/Capture mode
#[cfg(test)]
mod capture {
    use std::process::Stdio;

    use super::super::app::OutputPane;
    use super::{CAPTURE_LIMIT, describe_exit, owner_command, read_capped, run_of};

    #[test]
    fn the_pane_shows_stdout_then_stderr_under_a_heading() {
        let pane = OutputPane::new("t".into(), "a\nb\n", false, "not found\n", false);
        assert_eq!(pane.lines, ["a", "b", "── stderr ──", "not found"]);
    }

    #[test]
    fn a_cut_stream_says_so_and_silence_is_shown() {
        let pane = OutputPane::new("t".into(), "a\n", true, "", false);
        assert_eq!(pane.lines, ["a", "[output cut at 1 MiB]"]);
        let empty = OutputPane::new("t".into(), "", false, "", false);
        assert_eq!(empty.lines, ["(no output)"]);
    }

    #[test]
    fn output_past_the_limit_is_cut_and_drained() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "head -c 2000000 /dev/zero"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (kept, cut) = read_capped(child.stdout.take().unwrap()).join().unwrap();
        assert!(
            child.wait().unwrap().success(),
            "drained, so the writer never blocks"
        );
        assert_eq!(kept.len() as u64, CAPTURE_LIMIT);
        assert!(cut);
    }

    #[test]
    fn output_under_the_limit_is_kept_whole() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "printf hello"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let (kept, cut) = read_capped(child.stdout.take().unwrap()).join().unwrap();
        child.wait().unwrap();
        assert_eq!(kept, b"hello");
        assert!(!cut);
    }

    #[test]
    fn the_command_runs_in_the_target_with_its_arguments() {
        let dir = std::env::temp_dir();
        let output = owner_command(&run_of(&["sh", "-c", "pwd; echo \"$0\"", "one arg"], &dir))
            .output()
            .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        let canonical = dir.canonicalize().unwrap();
        assert_eq!(text, format!("{}\none arg\n", canonical.display()));
        assert_eq!(describe_exit(output.status), "exited 0");
    }

    #[test]
    fn a_failing_exit_reads_its_code() {
        let status = std::process::Command::new("sh")
            .args(["-c", "exit 3"])
            .status()
            .unwrap();
        assert_eq!(describe_exit(status), "exited 3");
    }
}

/// Requirement: browse-owner-actions/Detached mode
#[cfg(test)]
mod detached {
    use super::{run_detached, run_of};

    #[test]
    fn a_detached_program_starts_without_waiting_and_is_reaped() {
        use std::time::{Duration, Instant};

        let dir = std::env::temp_dir().join(format!("lattice-detached-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The program runs until released, or 30 s, so a run_detached that
        // waited returns only after it ended and fails the liveness check
        // below instead of hanging the suite.
        let script = "echo $$ > pid; i=0; \
            while [ ! -e release ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i + 1)); done";
        let message = run_detached(&run_of(&["sh", "-c", script], &dir));
        assert_eq!(message, "x started (sh)");

        let until = |deadline: Instant, done: &dyn Fn() -> bool| {
            while !done() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            done()
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        assert!(
            until(deadline, &|| std::fs::read_to_string(dir.join("pid"))
                .is_ok_and(|pid| pid.ends_with('\n'))),
            "the program never started"
        );
        let pid = std::fs::read_to_string(dir.join("pid")).unwrap();
        #[cfg(target_os = "linux")]
        {
            let state = || {
                std::fs::read_to_string(format!("/proc/{}/stat", pid.trim()))
                    .ok()
                    .and_then(|stat| stat.rsplit(") ").next().and_then(|s| s.chars().next()))
            };
            assert!(
                matches!(state(), Some(letter) if letter != 'Z'),
                "run_detached waited for the program: {:?}",
                state()
            );
            std::fs::write(dir.join("release"), "").unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            assert!(
                until(deadline, &|| state().is_none()),
                "the exited program was left as a zombie: {:?}",
                state()
            );
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = pid;
            std::fs::write(dir.join("release"), "").unwrap();
        }
    }

    #[test]
    fn a_program_that_cannot_start_says_so() {
        let message = run_detached(&run_of(&["/nonexistent/tool"], &std::env::temp_dir()));
        assert!(message.contains("could not start"), "{message}");
    }
}
