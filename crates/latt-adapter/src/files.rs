//! Path plumbing and file selection the Rust adapters share.

use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::{Document, Issue, Provenance, Severity, codes};

/// Return `path` joined to the current directory when relative, canonicalized
/// when it exists.
///
/// The adapters apply it to `--target`, so provenance relative to the target
/// and the paths handed to subprocesses agree whatever directory the adapter
/// was started from. A path that cannot be canonicalized (it does not exist, or
/// a component is unreadable) is returned joined but unresolved, so the caller
/// reports the missing target in its own terms.
///
/// # Errors
///
/// `path` is relative and the current directory cannot be read.
///
/// ```
/// use std::path::PathBuf;
///
/// let absolute = latt_adapter::absolute_path(PathBuf::from("."))?;
/// assert!(absolute.is_absolute());
/// assert_eq!(absolute, std::env::current_dir()?.canonicalize()?);
///
/// let missing = latt_adapter::absolute_path(PathBuf::from("/no/such/target"))?;
/// assert_eq!(missing, PathBuf::from("/no/such/target"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn absolute_path(path: PathBuf) -> io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()?.join(path)
    };
    Ok(std::fs::canonicalize(&absolute).unwrap_or(absolute))
}

/// Compile a file glob from `adapter.paths.files` into an anchored regex source.
///
/// `*` and `?` stay within one path segment; `**` crosses segments, and `**/`
/// also matches nothing, so `**/*.md` matches a file at the root. Every other
/// character is literal. Paths are matched with `/` separators.
///
/// ```
/// let pattern = regex::Regex::new(&latt_adapter::glob_regex("docs/**/*.md"))?;
/// assert!(pattern.is_match("docs/a.md"));
/// assert!(pattern.is_match("docs/x/y/a.md"));
/// assert!(!pattern.is_match("docs/a.mdx"));
/// assert_eq!(latt_adapter::glob_regex("a?.md"), r"^a[^/]\.md$");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn glob_regex(pattern: &str) -> String {
    let chars: Vec<char> = pattern.chars().collect();
    let mut regex = String::from("^");
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                index += 2;
                if chars.get(index) == Some(&'/') {
                    regex.push_str("(?:.*/)?");
                    index += 1;
                } else {
                    regex.push_str(".*");
                }
            }
            '*' => {
                regex.push_str("[^/]*");
                index += 1;
            }
            '?' => {
                regex.push_str("[^/]");
                index += 1;
            }
            character => {
                regex.push_str(&regex::escape(&character.to_string()));
                index += 1;
            }
        }
    }
    regex.push('$');
    regex
}

/// Return true when a symlink resolves to a file, or resolves to nothing.
///
/// A dangling link is kept so that reading it reports it, never skipped in
/// silence. A link to a directory is not a file.
///
/// ```
/// assert!(latt_adapter::is_file_or_dangling(std::path::Path::new("/no/such/link")));
/// assert!(!latt_adapter::is_file_or_dangling(&std::env::temp_dir()));
/// ```
#[doc(hidden)]
pub fn is_file_or_dangling(path: &Path) -> bool {
    std::fs::metadata(path).map_or(true, |target| target.is_file())
}

/// The compiled `adapter.paths.files` globs, kept beside their spelling so a
/// selection that matched nothing can name them.
#[derive(Debug)]
#[doc(hidden)]
pub struct FileMatcher {
    globs: Vec<String>,
    patterns: Vec<Regex>,
}

impl FileMatcher {
    /// Compile `globs` with [`glob_regex`].
    ///
    /// # Errors
    ///
    /// A glob compiles to a regex the `regex` crate refuses; the message names
    /// the glob.
    ///
    /// ```
    /// latt_adapter::FileMatcher::new(&["*.md", "docs/**/*.md"])?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[doc(hidden)]
    pub fn new(globs: &[impl AsRef<str>]) -> Result<Self, String> {
        let globs: Vec<String> = globs.iter().map(|glob| glob.as_ref().to_owned()).collect();
        let patterns = globs
            .iter()
            .map(|glob| {
                Regex::new(&glob_regex(glob))
                    .map_err(|error| format!("invalid file glob '{glob}' in profile: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { globs, patterns })
    }

    fn matches(&self, path: &str) -> bool {
        self.patterns.iter().any(|pattern| pattern.is_match(path))
    }
}

fn display_path(path: &Path, root: &Path) -> String {
    shown_name(path, root).map_or_else(
        || path.display().to_string(),
        |name| {
            let shown = name.to_string_lossy();
            // Only where `\` separates components; elsewhere it is a byte of the name.
            if std::path::MAIN_SEPARATOR == '\\' {
                shown.replace('\\', "/")
            } else {
                shown.into_owned()
            }
        },
    )
}

/// Name `path` as IDs and findings do: relative to `root`, or by its file name when
/// `path` is `root` itself.
fn shown_name<'p>(path: &'p Path, root: &Path) -> Option<&'p OsStr> {
    path.strip_prefix(root)
        .ok()
        .map(Path::as_os_str)
        .filter(|relative| !relative.is_empty())
        .or_else(|| path.file_name())
}

fn visit_directory(
    document: &mut Document,
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            let file = display_path(directory, root);
            document.parse_error(
                format!("could not read directory '{file}': {error}"),
                file,
                0,
            );
            return;
        }
    };

    for result in entries {
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                document.parse_error(
                    format!(
                        "could not read an entry in '{}': {error}",
                        directory.display()
                    ),
                    display_path(directory, root),
                    0,
                );
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                let file = display_path(&path, root);
                document.parse_error(format!("could not inspect '{file}': {error}"), file, 0);
                continue;
            }
        };
        if file_type.is_dir() {
            visit_directory(document, root, &path, files);
        } else if file_type.is_file() || (file_type.is_symlink() && is_file_or_dangling(&path)) {
            files.push(path);
        }
    }
}

/// Select the files under `target` that `matcher` matches, in shown-name order,
/// each paired with the name findings and provenance show for it.
///
/// A directory target is walked recursively without following symlinked
/// directories; a file target is matched by its file name. Unreadable input is
/// reported, never dropped: a missing target, an unreadable directory or entry,
/// and a matching path that is not valid UTF-8 are each a `PARSE_ERROR`. A
/// readable target where nothing matches yields one info `NO_MATCHING_FILES`
/// naming the globs and the target, so a mistyped glob never reads as an empty
/// register; when dropping a glob's leading directories that repeat the
/// target's own trailing ones would match a file beneath it, the finding also
/// names the directory above to target instead.
///
/// ```
/// use latt_adapter::{Document, FileMatcher, selected_files};
///
/// let target = std::env::temp_dir().join(format!("latt-adapter-files-{}", std::process::id()));
/// std::fs::create_dir_all(target.join("nested"))?;
/// std::fs::write(target.join("nested/b.md"), "")?;
/// std::fs::write(target.join("a.md"), "")?;
/// std::fs::write(target.join("c.txt"), "")?;
///
/// let mut document = Document::new();
/// let found = selected_files(&mut document, &target, &FileMatcher::new(&["**/*.md"])?);
/// let names: Vec<&str> = found.iter().map(|(_, name)| name.as_str()).collect();
/// assert_eq!(names, ["a.md", "nested/b.md"]);
/// assert!(document.findings.is_empty());
///
/// let none = selected_files(&mut document, &target, &FileMatcher::new(&["*.toml"])?);
/// std::fs::remove_dir_all(&target)?;
/// assert!(none.is_empty());
/// assert_eq!(document.findings[0].code, "NO_MATCHING_FILES");
/// assert!(document.findings[0].message.starts_with(r#"no files matched adapter.paths.files ["*.toml"] under target '"#));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn selected_files(
    document: &mut Document,
    target: &Path,
    matcher: &FileMatcher,
) -> Vec<(PathBuf, String)> {
    let mut paths = Vec::new();
    let readable = target.is_dir() || target.is_file();
    if target.is_dir() {
        visit_directory(document, target, target, &mut paths);
    } else if target.is_file() {
        paths.push(target.to_owned());
    } else {
        document.parse_error(
            format!(
                "target '{}' does not exist or is unreadable",
                target.display()
            ),
            target.display().to_string(),
            0,
        );
    }

    let named: Vec<(PathBuf, String)> = paths
        .into_iter()
        .map(|path| {
            let display = display_path(&path, target);
            (path, display)
        })
        .collect();
    let mut selected: Vec<_> = named
        .iter()
        .filter(|(_, display)| matcher.matches(display))
        .cloned()
        .collect();
    if readable && selected.is_empty() {
        let mut message = format!(
            "no files matched adapter.paths.files {:?} under target '{}'; the patterns are relative to it",
            matcher.globs,
            target.display()
        );
        if let Some((glob, up)) = retarget_hint(target, &matcher.globs, &named) {
            message.push_str(&format!(
                "; '{glob}' matches under '{up}': use --target {up}"
            ));
        }
        document.findings.push(Issue::new(
            Severity::Info,
            codes::NO_MATCHING_FILES,
            message,
            Provenance::new_file("."),
        ));
    }
    // The path breaks ties, so two names shown alike keep one order.
    selected.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    selected.retain(|(path, display)| {
        let Some(name) = shown_name(path, target).filter(|name| name.to_str().is_none()) else {
            return true;
        };
        let message = format!("{display}: path is not valid UTF-8: {name:?}");
        document.parse_error(message, display.clone(), 0);
        false
    });
    selected
}

/// The directory to target when a glob's leading segments repeat the target's
/// own trailing ones: the first glob that matches a walked file once those
/// segments are dropped, with the ancestor as many levels up.
///
/// Bounded on purpose: the walk is not repeated, only the matcher is re-run
/// over names already beneath the target, and only for the segment counts that
/// literally agree with the target's path. The `.`, `..` and glob-bearing
/// segments stop the comparison, so `**/x.md` never climbs.
fn retarget_hint(
    target: &Path,
    globs: &[String],
    named: &[(PathBuf, String)],
) -> Option<(String, String)> {
    let canonical = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    let trailing: Vec<String> = canonical
        .components()
        .filter_map(|component| match component {
            std::path::Component::Normal(segment) => Some(segment.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    for glob in globs {
        let segments: Vec<&str> = glob.split('/').collect();
        let directories = &segments[..segments.len().saturating_sub(1)];
        let literal = directories
            .iter()
            .take_while(|segment| {
                !segment.contains(['*', '?', '[']) && !matches!(**segment, "." | "..")
            })
            .count();
        for dropped in 1..=literal.min(trailing.len()) {
            let agree = segments[..dropped]
                .iter()
                .zip(&trailing[trailing.len() - dropped..])
                .all(|(glob_segment, target_segment)| *glob_segment == target_segment);
            if !agree {
                continue;
            }
            let stripped = segments[dropped..].join("/");
            let Ok(matcher) = FileMatcher::new(&[stripped]) else {
                continue;
            };
            if named.iter().any(|(_, display)| matcher.matches(display)) {
                return Some((glob.clone(), ancestor_display(target, dropped)));
            }
        }
    }
    None
}

/// `target` with `levels` trailing components removed, as the user would write
/// it on the command line: `docs` one level up is `.`, not an empty string.
fn ancestor_display(target: &Path, levels: usize) -> String {
    let mut up = target.to_path_buf();
    for _ in 0..levels {
        if !up.pop() {
            break;
        }
    }
    if up.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        up.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use regex::Regex;

    use super::{Document, FileMatcher, ancestor_display, glob_regex, selected_files};

    /// A fresh directory per test under the system temp dir.
    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("latt-adapter-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn only_finding(target: &Path, glob: &str) -> String {
        let mut document = Document::new();
        let found = selected_files(&mut document, target, &FileMatcher::new(&[glob]).unwrap());
        assert!(found.is_empty(), "{found:?}");
        let [finding] = document.findings.as_slice() else {
            panic!("one finding: {:?}", document.findings);
        };
        assert_eq!(finding.code, "NO_MATCHING_FILES");
        finding.message.clone()
    }

    // Requirement: adapter-md/No matching files names the target and a fix
    // Requirement: adapter-toml/No matching files names the target and a fix
    #[test]
    fn no_matching_files_names_the_directory_above_a_target_one_level_too_deep() {
        for (dir, glob, file) in [
            ("docs", "docs/*.md", "requirements.md"),
            ("registers", "registers/*.toml", "requirements.toml"),
        ] {
            let root = scratch(dir);
            let target = root.join(dir);
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join(file), "").unwrap();
            let message = only_finding(&target, glob);
            assert!(message.contains(&format!("[{glob:?}]")), "{message}");
            assert!(
                message.contains(&format!("target '{}'", target.display())),
                "{message}"
            );
            assert!(message.contains("relative to"), "{message}");
            assert!(
                message.contains(&format!("use --target {}", root.display())),
                "{message}"
            );
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn no_matching_files_offers_no_target_when_dropping_segments_matches_nothing() {
        let root = scratch("nofix");
        let target = root.join("docs");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("notes.txt"), "").unwrap();
        for glob in ["docs/*.md", "registers/*.toml", "*.toml"] {
            let message = only_finding(&target, glob);
            assert!(message.contains(&format!("[{glob:?}]")), "{message}");
            assert!(
                message.contains(&format!("target '{}'", target.display())),
                "{message}"
            );
            assert!(!message.contains("use --target"), "{message}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn no_matching_files_climbs_as_many_levels_as_the_segments_dropped() {
        let root = scratch("deep");
        let target = root.join("a").join("b");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("r.md"), "").unwrap();
        let message = only_finding(&target, "a/b/*.md");
        assert!(
            message.contains(&format!("use --target {}", root.display())),
            "{message}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_ancestor_of_a_bare_target_is_the_working_directory() {
        assert_eq!(ancestor_display(Path::new("docs"), 1), ".");
        assert_eq!(ancestor_display(Path::new("a/b"), 1), "a");
        assert_eq!(ancestor_display(Path::new("a/b"), 2), ".");
        assert_eq!(ancestor_display(Path::new("/x/y/docs"), 1), "/x/y");
    }

    #[test]
    fn recursive_glob_also_matches_a_root_file() {
        let pattern = Regex::new(&glob_regex("**/*.md")).unwrap();
        assert!(pattern.is_match("one.md"));
        assert!(pattern.is_match("nested/one.md"));
        assert!(!pattern.is_match("one.toml"));
    }
}
