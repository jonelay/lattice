//! Bounded subprocess execution shared by the core and the adapters.
//!
//! Every subprocess Lattice spawns runs under a wall-clock bound. On expiry the child
//! is killed and reaped, and the caller reports the expiry through the same failure
//! path it uses for a non-zero exit. Dropping a [`BoundedChild`] kills a still-running
//! process, so an early return or a panic never leaks one.

use std::io::Read;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// First pause between exit checks once both pipes have closed; it doubles
/// up to [`POLL_CEILING`], so a child that closes its pipes and lives on
/// costs one check per 10 ms, not one per 50 µs.
const POLL_FLOOR: Duration = Duration::from_micros(50);
const POLL_CEILING: Duration = Duration::from_millis(10);

/// The pauses between exit checks: [`POLL_FLOOR`], doubling to [`POLL_CEILING`].
fn pause_schedule() -> impl Iterator<Item = Duration> {
    std::iter::successors(Some(POLL_FLOOR), |pause| {
        Some((*pause * 2).min(POLL_CEILING))
    })
}

/// Why a bounded run produced no [`Output`].
///
/// Resource budgets beyond time are future work, so a variant can be added in
/// a minor release; a `match` needs a wildcard arm.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BoundedError {
    /// The child was still running when the bound expired and has been killed.
    #[error("{program} exceeded {} s", format_seconds(*bound))]
    TimedOut {
        /// Program name as given to [`Command::new`].
        program: String,
        /// The bound that expired.
        bound: Duration,
    },
    /// Spawning, waiting on, or reading a pipe of the child failed.
    ///
    /// A read failure is an error, not empty output: silence would read as
    /// "the program wrote nothing".
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

fn format_seconds(bound: Duration) -> String {
    if bound.subsec_nanos() == 0 {
        bound.as_secs().to_string()
    } else {
        format!("{:.1}", bound.as_secs_f64())
    }
}

/// Run `command` to completion within `bound`, capturing stdout and stderr.
///
/// This is the bounded replacement for [`Command::output`]. A non-zero exit is
/// not an error here; it is in the returned [`Output`]'s status.
///
/// # Errors
///
/// [`BoundedError::TimedOut`] when the child outlives `bound` (it is killed and
/// reaped first), and [`BoundedError::Io`] when it cannot be spawned, waited on,
/// or read: a pipe read that fails is reported, never returned as empty output.
///
/// ```
/// # #[cfg(unix)] {
/// use std::process::Command;
/// use std::time::Duration;
/// use latt_adapter::process::{BoundedError, run_bounded};
///
/// let mut sleep = Command::new("sleep");
/// sleep.arg("5");
/// let error = run_bounded(sleep, Duration::from_millis(50)).unwrap_err();
/// assert!(matches!(error, BoundedError::TimedOut { .. }));
///
/// let output = run_bounded(Command::new("false"), Duration::from_secs(5))?;
/// assert!(!output.status.success(), "a failed exit is output, not an error");
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn run_bounded(command: Command, bound: Duration) -> Result<Output, BoundedError> {
    spawn_bounded(command, bound)?.wait_output()
}

/// Spawn `command` with piped stdout and stderr under `bound`, measured from now.
///
/// Use this instead of [`run_bounded`] when the caller must write to the child's
/// stdin or hand its stdout to another process before waiting.
///
/// # Errors
///
/// The spawn error when the program cannot be started.
pub fn spawn_bounded(mut command: Command, bound: Duration) -> std::io::Result<BoundedChild> {
    let program = command.get_program().to_string_lossy().into_owned();
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    Ok(BoundedChild {
        child,
        program,
        deadline: deadline_after(bound),
        bound,
    })
}

/// The instant `bound` from now, saturating to a century away.
///
/// A profile may declare any positive timeout; one past what the clock can
/// represent means "no practical bound", not a panic outside the exit contract.
fn deadline_after(bound: Duration) -> Instant {
    const CENTURY: Duration = Duration::from_secs(100 * 365 * 24 * 60 * 60);
    let now = Instant::now();
    now.checked_add(bound)
        .or_else(|| now.checked_add(CENTURY))
        .unwrap_or(now)
}

/// Why a labelled subprocess run produced no [`Output`], split by who must act.
#[derive(Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum Failure {
    /// The program could not be started: a broken setup, not unreadable input,
    /// so the adapter exits non-zero instead of emitting a document.
    Unrunnable(String),
    /// The finding text for a run that started and then expired or could not be
    /// read, or that the caller refused before starting it.
    Failed(String),
}

impl Failure {
    /// Return the text either way, for a caller that reports both alike.
    ///
    /// ```
    /// use latt_adapter::process::Failure;
    ///
    /// assert_eq!(Failure::Failed("git ls-tree exceeded 1 s".into()).into_message(), "git ls-tree exceeded 1 s");
    /// ```
    #[must_use]
    #[doc(hidden)]
    pub fn into_message(self) -> String {
        match self {
            Self::Unrunnable(message) | Self::Failed(message) => message,
        }
    }
}

/// Describe a [`BoundedError`] as a finding naming the command by `label`.
///
/// `label` is the command line a user would recognise, such as `git ls-tree`:
/// an expiry reads `{label} exceeded {bound} s`, with the bound printed as
/// [`BoundedError`]'s own message prints it, and any other error reads
/// `could not run {label}: {error}`.
///
/// ```
/// use std::time::Duration;
/// use latt_adapter::process::{BoundedError, describe};
///
/// let expired = |bound| BoundedError::TimedOut { program: "git".into(), bound };
/// assert_eq!(describe("git cat-file --batch", &expired(Duration::from_secs(1))),
///            "git cat-file --batch exceeded 1 s");
/// assert_eq!(describe("git ls-tree", &expired(Duration::from_millis(500))),
///            "git ls-tree exceeded 0.5 s");
/// let broken = BoundedError::Io(std::io::Error::other("pipe closed"));
/// assert_eq!(describe("git cat-file --batch", &broken), "could not run git cat-file --batch: pipe closed");
/// ```
#[doc(hidden)]
pub fn describe(label: &str, error: &BoundedError) -> String {
    match error {
        BoundedError::TimedOut { bound, .. } => {
            format!("{label} exceeded {} s", format_seconds(*bound))
        }
        error => format!("could not run {label}: {error}"),
    }
}

/// Run `command` to completion within `bound`, splitting a failure into
/// [`Failure`] and naming the command by `label` as [`describe`] does.
///
/// A non-zero exit is not a failure here; it is in the returned [`Output`]'s
/// status, for the caller to describe with the program's stderr.
///
/// # Errors
///
/// [`Failure::Unrunnable`] naming the program when it cannot be spawned, and
/// [`Failure::Failed`] with [`describe`]'s text when the run expires or its
/// output cannot be read.
///
/// ```
/// # #[cfg(unix)] {
/// use std::process::Command;
/// use std::time::Duration;
/// use latt_adapter::process::{Failure, run_labelled};
///
/// let mut sleep = Command::new("sleep");
/// sleep.arg("5");
/// assert_eq!(run_labelled(sleep, "sleep 5", Duration::from_millis(100)).unwrap_err(),
///            Failure::Failed("sleep 5 exceeded 0.1 s".into()));
///
/// let missing = run_labelled(Command::new("no-such-program-here"), "x", Duration::from_secs(1));
/// assert!(matches!(missing, Err(Failure::Unrunnable(m)) if m.starts_with("could not run no-such-program-here: ")));
///
/// let output = run_labelled(Command::new("false"), "false", Duration::from_secs(5))
///     .map_err(Failure::into_message)?;
/// assert!(!output.status.success());
/// # }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[doc(hidden)]
pub fn run_labelled(command: Command, label: &str, bound: Duration) -> Result<Output, Failure> {
    let program = command.get_program().to_string_lossy().into_owned();
    let child = spawn_bounded(command, bound)
        .map_err(|error| Failure::Unrunnable(format!("could not run {program}: {error}")))?;
    child
        .wait_output()
        .map_err(|error| Failure::Failed(describe(label, &error)))
}

/// A spawned child that is killed when its bound expires or when it is dropped.
#[derive(Debug)]
#[must_use = "dropping the handle kills the child"]
pub struct BoundedChild {
    child: Child,
    program: String,
    deadline: Instant,
    bound: Duration,
}

impl BoundedChild {
    /// Operating-system process identifier of the child.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// The child's stdin, when the command piped it.
    pub fn stdin(&mut self) -> Option<&mut ChildStdin> {
        self.child.stdin.as_mut()
    }

    /// Take the child's stdin so a writer thread can feed it.
    ///
    /// A large write from the calling thread would block once the pipe fills,
    /// before the bound is ever polled; a writer thread keeps the bound honest.
    /// The caller owns EOF from here: [`wait_output`](Self::wait_output) can no
    /// longer close the pipe, so the handle must be dropped once the write is
    /// done or a child that reads to end-of-input waits out the whole bound.
    ///
    /// ```
    /// # #[cfg(unix)] {
    /// use std::io::Write;
    /// use std::process::{Command, Stdio};
    /// use std::time::Duration;
    /// use latt_adapter::process::{BoundedError, spawn_bounded};
    ///
    /// let cat = || { let mut c = Command::new("cat"); c.stdin(Stdio::piped()); c };
    ///
    /// let mut child = spawn_bounded(cat(), Duration::from_secs(5))?;
    /// let mut stdin = child.take_stdin().ok_or("stdin is piped")?;
    /// let writer = std::thread::spawn(move || {
    ///     stdin.write_all(b"fed through the pipe")
    ///     // `stdin` drops here: that is the child's end-of-input.
    /// });
    /// let output = child.wait_output()?;
    /// writer.join().expect("the writer thread does not panic")?;
    /// assert_eq!(output.stdout, b"fed through the pipe");
    ///
    /// // Keep the handle instead and `cat` never sees end-of-input.
    /// let mut child = spawn_bounded(cat(), Duration::from_millis(100))?;
    /// let kept = child.take_stdin().ok_or("stdin is piped")?;
    /// let error = child.wait_output().unwrap_err();
    /// assert!(matches!(error, BoundedError::TimedOut { .. }), "{error}");
    /// drop(kept);
    /// # }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    /// Take the child's stdout so another process can read it.
    ///
    /// After this, [`wait_output`](Self::wait_output) returns empty stdout.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    /// Close stdin unless it was taken, drain stdout and stderr, and wait for
    /// exit within the bound.
    ///
    /// On expiry the child is killed and reaped before `TimedOut` is returned.
    /// The bound also covers the drain: a descendant that inherited the pipe
    /// and outlives the child would otherwise hold the read open indefinitely.
    ///
    /// The pipes reaching end-of-file is the wake-up: the caller blocks on the
    /// reader threads' channel until then or the deadline, whichever is first,
    /// with no fixed polling quantum after the child has already exited.
    ///
    /// # Errors
    ///
    /// [`BoundedError::TimedOut`] when the bound expires first, and
    /// [`BoundedError::Io`] when a pipe read or the wait itself fails — a failed
    /// read is never reported as empty output.
    pub fn wait_output(mut self) -> Result<Output, BoundedError> {
        drop(self.child.stdin.take());
        let Some((stdout, stderr)) = drain_by(
            self.child.stdout.take(),
            self.child.stderr.take(),
            self.deadline,
        )?
        else {
            // A pipe still open at the deadline: the child is running, or a
            // descendant that inherited the pipe outlives it. Both are the
            // bound expiring; the kill is a no-op on an exited child.
            self.kill_and_reap();
            return Err(self.timed_out());
        };
        let status = self.wait_bounded()?;
        Ok(Output {
            status,
            stdout,
            stderr,
        })
    }

    fn timed_out(&mut self) -> BoundedError {
        BoundedError::TimedOut {
            program: std::mem::take(&mut self.program),
            bound: self.bound,
        }
    }

    /// Collect the exit status once the pipes have closed.
    ///
    /// A process closes its descriptors before it becomes waitable, so the
    /// first check can land in that gap; the pause starts far below the old
    /// 10 ms quantum and only climbs to it for a child that closed both pipes
    /// and lives on, which nothing but polling can bound without a signal.
    /// `Child::wait` on a helper thread is not an option: the kill at the
    /// deadline needs the same `&mut Child` that the blocked wait holds.
    fn wait_bounded(&mut self) -> Result<ExitStatus, BoundedError> {
        let mut pauses = pause_schedule();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            let remaining = self.deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                self.kill_and_reap();
                return Err(self.timed_out());
            }
            let pause = pauses.next().expect("the schedule never ends");
            std::thread::sleep(pause.min(remaining));
        }
    }

    fn kill_and_reap(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for BoundedChild {
    fn drop(&mut self) {
        // Anything but a confirmed exit gets the kill: an error from `try_wait`
        // must not leave a live child behind.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            self.kill_and_reap();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pipe {
    Stdout,
    Stderr,
}

type Drained = (Pipe, std::io::Result<Vec<u8>>);

/// Read a pipe to end-of-file on its own thread and report the result.
///
/// The send fails only when the receiver has given up at the deadline; the
/// thread then ends with the process that held the pipe open.
fn drain<R: Read + Send + 'static>(pipe: Pipe, mut reader: R, report: Sender<Drained>) {
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        let result = reader.read_to_end(&mut buffer).map(|_| buffer);
        let _ = report.send((pipe, result));
    });
}

/// Read both pipes to end-of-file by `deadline`, blocking on the readers'
/// channel rather than polling; `None` when a pipe is still open at the
/// deadline, in which case its thread is left to end when the process holding
/// the pipe does. A read failure is the error itself, not an empty buffer.
fn drain_by<O, E>(
    stdout: Option<O>,
    stderr: Option<E>,
    deadline: Instant,
) -> std::io::Result<Option<(Vec<u8>, Vec<u8>)>>
where
    O: Read + Send + 'static,
    E: Read + Send + 'static,
{
    let (report, results) = mpsc::channel();
    let mut pending = 0;
    if let Some(reader) = stdout {
        drain(Pipe::Stdout, reader, report.clone());
        pending += 1;
    }
    if let Some(reader) = stderr {
        drain(Pipe::Stderr, reader, report.clone());
        pending += 1;
    }
    drop(report);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    while pending > 0 {
        match results.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((Pipe::Stdout, bytes)) => out = bytes?,
            Ok((Pipe::Stderr, bytes)) => err = bytes?,
            Err(RecvTimeoutError::Timeout) => return Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(std::io::Error::other(
                    "the pipe reader ended without a result",
                ));
            }
        }
        pending -= 1;
    }
    Ok(Some((out, err)))
}

#[cfg(all(test, unix))]
mod tests {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use super::{
        BoundedError, POLL_CEILING, POLL_FLOOR, describe, drain_by, pause_schedule, run_bounded,
        spawn_bounded,
    };

    /// A pipe whose every read fails, as a broken pipe reader does.
    struct FailingRead;

    impl std::io::Read for FailingRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("pipe read failed"))
        }
    }

    #[test]
    fn a_failed_pipe_read_is_reported_not_emptied() {
        let deadline = Instant::now() + Duration::from_secs(5);
        let error = drain_by(Some(FailingRead), Some(std::io::empty()), deadline)
            .expect_err("the read failed");
        assert_eq!(error.to_string(), "pipe read failed");
        let error = BoundedError::from(error);
        assert!(matches!(error, BoundedError::Io(_)), "{error}");
    }

    #[test]
    fn drains_both_pipes_in_either_order() {
        let deadline = Instant::now() + Duration::from_secs(5);
        let drained = drain_by(Some(&b"out"[..]), Some(&b"err"[..]), deadline)
            .expect("reads succeed")
            .expect("both pipes close");
        assert_eq!(drained, (b"out".to_vec(), b"err".to_vec()));
        let drained = drain_by(None::<&[u8]>, Some(&b"err"[..]), deadline)
            .expect("reads succeed")
            .expect("the one pipe closes");
        assert_eq!(drained, (Vec::new(), b"err".to_vec()));
    }

    #[test]
    fn a_pipe_held_open_past_the_deadline_is_none() {
        // A reader that never reaches end-of-file stands in for a descendant
        // that inherited the pipe; it blocks for far longer than the bound.
        struct Held;
        impl std::io::Read for Held {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                std::thread::sleep(Duration::from_secs(30));
                Ok(0)
            }
        }
        let started = Instant::now();
        let deadline = started + Duration::from_millis(200);
        let drained =
            drain_by(Some(Held), Some(std::io::empty()), deadline).expect("no read error");
        assert!(drained.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn exit_is_seen_without_a_polling_quantum() {
        // A child is waitable microseconds after its pipes close, so the wait
        // must start far below the 10 ms quantum it once slept outright and
        // climb only for a child that lives on. Asserted on the schedule the
        // wait consumes, not on wall time, which a loaded runner stretches.
        let pauses: Vec<Duration> = pause_schedule().take(16).collect();
        assert_eq!(pauses[0], POLL_FLOOR);
        assert!(POLL_FLOOR * 100 <= POLL_CEILING);
        for pair in pauses.windows(2) {
            assert_eq!(pair[1], (pair[0] * 2).min(POLL_CEILING));
        }
        assert_eq!(pauses.last(), Some(&POLL_CEILING));
        let output = run_bounded(Command::new("true"), Duration::from_secs(5)).unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn a_bound_beyond_the_clock_does_not_panic() {
        let output = run_bounded(Command::new("true"), Duration::MAX).unwrap();
        assert!(output.status.success());
    }

    fn process_is_gone(pid: u32) -> bool {
        !Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .expect("kill is available")
            .status
            .success()
    }

    #[test]
    fn completes_under_the_bound_with_full_output() {
        let mut command = Command::new("sh");
        command.args(["-c", "echo out; echo err >&2"]);
        let output = run_bounded(command, Duration::from_secs(5)).expect("sh runs");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
    }

    #[test]
    fn exceeding_the_bound_is_timed_out_and_reaped() {
        let mut command = Command::new("sleep");
        command.arg("30");
        let child = spawn_bounded(command, Duration::from_secs(1)).expect("sleep spawns");
        let pid = child.id();
        let started = Instant::now();
        let error = child.wait_output().expect_err("bound expires");
        assert!(started.elapsed() < Duration::from_secs(5));
        match &error {
            BoundedError::TimedOut { program, bound } => {
                assert_eq!(program, "sleep");
                assert_eq!(*bound, Duration::from_secs(1));
            }
            BoundedError::Io(error) => panic!("unexpected io error: {error}"),
        }
        assert_eq!(error.to_string(), "sleep exceeded 1 s");
        assert!(process_is_gone(pid));
    }

    #[test]
    fn a_descendant_holding_the_pipe_does_not_block_past_the_bound() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 & exit 0"]);
        let started = Instant::now();
        let error = run_bounded(command, Duration::from_secs(1)).expect_err("pipe stays open");
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(matches!(error, BoundedError::TimedOut { .. }), "{error}");
    }

    #[test]
    fn dropping_the_child_kills_it() {
        let mut command = Command::new("sleep");
        command.arg("30");
        let child = spawn_bounded(command, Duration::from_secs(30)).expect("sleep spawns");
        let pid = child.id();
        assert!(!process_is_gone(pid));
        drop(child);
        assert!(process_is_gone(pid));
    }

    #[test]
    fn describe_prints_the_bound_as_bounded_error_does() {
        for (bound, shown) in [
            (Duration::from_secs(1), "1"),
            (Duration::from_millis(1500), "1.5"),
        ] {
            let error = BoundedError::TimedOut {
                program: "git".to_owned(),
                bound,
            };
            assert_eq!(
                describe("git cat-file --batch", &error),
                format!("git cat-file --batch exceeded {shown} s")
            );
            assert_eq!(error.to_string(), format!("git exceeded {shown} s"));
        }
    }
}
