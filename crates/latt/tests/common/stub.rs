//! Executable shell stubs that are safe to run from parallel tests.
//!
//! Compiled into the integration suites through `common` and into the binary's
//! unit tests through `#[path]`, so there is one copy of the retry.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Errno for "text file busy" on Linux and the BSDs.
const ETXTBSY: i32 = 26;

/// Write `script` to `path` as an executable stub, and return once it can run.
///
/// A process another test thread forks while the file is open for writing
/// inherits that handle until it execs, and the kernel refuses to run a file
/// open for writing (ETXTBSY). The handle here is closed before the probe, so
/// once one probe run starts no stray handle remains and every later run is
/// safe. The probe line inserted after the shebang makes that run exit at once.
///
/// # Panics
///
/// Panics when `script` has no shebang line, when the file cannot be written,
/// or when the stub stays busy for a second.
pub fn write_stub(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    let (shebang, body) = script
        .split_once('\n')
        .filter(|(first, _)| first.starts_with("#!"))
        .expect("a stub script starts with a shebang line");
    let text = format!("{shebang}\n[ -z \"$LATTICE_STUB_PROBE\" ] || exit 0\n{body}");
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..50 {
        match Command::new(path)
            .env("LATTICE_STUB_PROBE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Err(error) if error.raw_os_error() == Some(ETXTBSY) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(error) => panic!("stub {} cannot run: {error}", path.display()),
            Ok(status) => {
                assert!(
                    status.success(),
                    "stub {} probe failed: {status}",
                    path.display()
                );
                return;
            }
        }
    }
    panic!("stub {} stayed busy for writing", path.display());
}
