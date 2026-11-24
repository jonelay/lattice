//! Bounded subprocess execution shared by the core and the adapters.
//!
//! Every subprocess Lattice spawns runs under a wall-clock bound. On expiry the child
//! is killed and reaped, and the caller reports the expiry through the same failure
//! path it uses for a non-zero exit. Dropping a [`BoundedChild`] kills a still-running
//! process, so an early return or a panic never leaks one.

use std::io::Read;
use std::ops::ControlFlow;
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
        limits: Limits::default(),
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
    limits: Limits,
}

/// What a run keeps of each pipe; `None` keeps everything.
#[derive(Clone, Copy, Debug, Default)]
struct Limits {
    stdout: Option<usize>,
    stderr_tail: Option<usize>,
}

impl BoundedChild {
    /// Fail the run when the child writes more than `bytes` to stdout.
    ///
    /// The child is killed and reaped and [`wait_output`](Self::wait_output)
    /// returns [`BoundedError::Io`] with [`std::io::ErrorKind::FileTooLarge`]:
    /// output is never returned cut short. The same error covers an allocation
    /// that fails while the buffer grows. Without this call stdout is unbounded.
    ///
    /// ```
    /// # #[cfg(unix)] {
    /// use std::process::Command;
    /// use std::time::Duration;
    /// use latt_adapter::process::{BoundedError, spawn_bounded};
    ///
    /// let child = spawn_bounded(Command::new("yes"), Duration::from_secs(30))?
    ///     .output_limit(4096);
    /// let BoundedError::Io(error) = child.wait_output().unwrap_err() else {
    ///     panic!("an io error");
    /// };
    /// assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
    /// # }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn output_limit(mut self, bytes: usize) -> Self {
        self.limits.stdout = Some(bytes);
        self
    }

    /// Keep only the last `bytes` of the child's stderr.
    ///
    /// stderr is diagnostics, so a flood never fails the run; the earlier bytes
    /// are dropped as the later ones arrive. Without this call stderr is kept whole.
    pub fn stderr_tail(mut self, bytes: usize) -> Self {
        self.limits.stderr_tail = Some(bytes);
        self
    }

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
        let drained = drain_by(
            self.child.stdout.take(),
            self.child.stderr.take(),
            self.deadline,
            self.limits,
        );
        let drained = match drained {
            Ok(drained) => drained,
            Err(error) => {
                // Over the output limit or a failed read: the readers are joined
                // already, so stop the child before it blocks on a closed pipe.
                self.kill_and_reap();
                return Err(error.into());
            }
        };
        let Some((stdout, stderr)) = drained else {
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

/// How long a reader waits for data between looks at its cancel flag, so a
/// cancelled reader ends within this and an idle one wakes only this often.
const CANCEL_TICK: Duration = Duration::from_millis(20);

/// The most a cancelled reader still drains. A pipe holds at most 1 MiB on
/// Linux by default, so this keeps everything a finished writer left; a writer
/// still going (its stdout over the limit while it floods stderr) cannot keep
/// the reader, and with it the kill that waits on the join, from ending.
const CANCELLED_DRAIN_BYTES: usize = 4 << 20;

/// A pipe that can be waited on without reading it, which is what lets a
/// reader thread stop while a descendant still holds the write end open.
#[doc(hidden)]
pub trait Pollable {
    /// True when a read would not block: data is buffered, or the write end has
    /// closed. False when `wait` passed with neither.
    ///
    /// # Errors
    ///
    /// The operating system's error when the wait itself fails.
    fn readable_within(&self, wait: Duration) -> std::io::Result<bool>;
}

#[cfg(unix)]
fn poll_readable(fd: std::os::fd::BorrowedFd<'_>, wait: Duration) -> std::io::Result<bool> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::io::Errno;

    let timeout = Timespec {
        tv_sec: i64::try_from(wait.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(wait.subsec_nanos()),
    };
    loop {
        let mut fds = [PollFd::new(&fd, PollFlags::IN)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(ready) => return Ok(ready > 0),
            Err(Errno::INTR) => {}
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(unix)]
impl Pollable for ChildStdout {
    fn readable_within(&self, wait: Duration) -> std::io::Result<bool> {
        poll_readable(std::os::fd::AsFd::as_fd(self), wait)
    }
}

#[cfg(unix)]
impl Pollable for std::process::ChildStderr {
    fn readable_within(&self, wait: Duration) -> std::io::Result<bool> {
        poll_readable(std::os::fd::AsFd::as_fd(self), wait)
    }
}

/// Without `poll(2)` a read cannot be cancelled: it blocks until end-of-file.
#[cfg(not(unix))]
impl Pollable for ChildStdout {
    fn readable_within(&self, _: Duration) -> std::io::Result<bool> {
        Ok(true)
    }
}

#[cfg(not(unix))]
impl Pollable for std::process::ChildStderr {
    fn readable_within(&self, _: Duration) -> std::io::Result<bool> {
        Ok(true)
    }
}

/// Feed `reader`'s bytes to `take` until end-of-file, or until `cancel` is set
/// and nothing more is buffered or 4 MiB has been read since.
///
/// A cancelled read still delivers what the pipe already holds, so output a
/// finished program wrote is kept even while a descendant holds the pipe open;
/// a writer that never pauses cannot hold a cancelled read open.
///
/// # Errors
///
/// A failed wait or read.
#[doc(hidden)]
pub fn read_until_cancelled<R: Read + Pollable>(
    reader: &mut R,
    cancel: &AtomicBool,
    mut take: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    read_until_stopped(reader, cancel, |bytes| {
        take(bytes);
        ControlFlow::Continue(())
    })
}

/// [`read_until_cancelled`], ending early when `take` breaks.
fn read_until_stopped<R: Read + Pollable>(
    reader: &mut R,
    cancel: &AtomicBool,
    mut take: impl FnMut(&[u8]) -> ControlFlow<()>,
) -> std::io::Result<()> {
    let mut chunk = [0u8; 8192];
    let mut drained_after_cancel = 0usize;
    loop {
        let cancelled = cancel.load(Ordering::Acquire);
        if cancelled && drained_after_cancel >= CANCELLED_DRAIN_BYTES {
            return Ok(());
        }
        let wait = if cancelled {
            Duration::ZERO
        } else {
            CANCEL_TICK
        };
        if !reader.readable_within(wait)? {
            if cancelled {
                return Ok(());
            }
            continue;
        }
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if cancelled {
                    drained_after_cancel += count;
                }
                if take(&chunk[..count]).is_break() {
                    return Ok(());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

/// The reader threads of one bounded run. Dropping it cancels and joins them,
/// which takes at most [`CANCEL_TICK`]: no reader outlives the call.
struct Readers {
    cancel: Arc<AtomicBool>,
    handles: Vec<std::thread::JoinHandle<()>>,
}

impl Readers {
    /// Read a pipe to end-of-file or cancellation on its own thread and report
    /// the result; the send fails only when the receiver has already gone.
    fn spawn<R: Read + Pollable + Send + 'static>(
        &mut self,
        pipe: Pipe,
        mut reader: R,
        report: Sender<Drained>,
        limits: Limits,
    ) {
        let cancel = Arc::clone(&self.cancel);
        self.handles.push(std::thread::spawn(move || {
            let mut buffer = Vec::new();
            let mut refusal = None;
            let result = read_until_stopped(&mut reader, &cancel, |bytes| match pipe {
                Pipe::Stdout => match keep_within(&mut buffer, bytes, limits.stdout) {
                    Ok(()) => ControlFlow::Continue(()),
                    Err(error) => {
                        refusal = Some(error);
                        ControlFlow::Break(())
                    }
                },
                Pipe::Stderr => {
                    keep_tail(&mut buffer, bytes, limits.stderr_tail);
                    ControlFlow::Continue(())
                }
            });
            let result = match (refusal, result) {
                (Some(error), _) => Err(error),
                (None, Ok(())) => {
                    if let (Pipe::Stderr, Some(tail)) = (pipe, limits.stderr_tail) {
                        trim_to_tail(&mut buffer, tail);
                    }
                    Ok(buffer)
                }
                (None, Err(error)) => Err(error),
            };
            let _ = report.send((pipe, result));
        }));
    }
}

/// Append `bytes` to `buffer` unless that passes `limit` or the allocation fails.
///
/// Both refusals are [`std::io::ErrorKind::FileTooLarge`]: nothing is appended
/// past the limit, so the buffer is never a truncated answer.
fn keep_within(buffer: &mut Vec<u8>, bytes: &[u8], limit: Option<usize>) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    if let Some(limit) = limit
        && buffer.len().saturating_add(bytes.len()) > limit
    {
        return Err(Error::new(
            ErrorKind::FileTooLarge,
            format!("output passed {limit} bytes"),
        ));
    }
    buffer.try_reserve(bytes.len()).map_err(|error| {
        Error::new(
            ErrorKind::FileTooLarge,
            format!(
                "no memory to hold more than {} bytes of output: {error}",
                buffer.len()
            ),
        )
    })?;
    buffer.extend_from_slice(bytes);
    Ok(())
}

/// Append `bytes`, then drop the oldest ones once the buffer is twice `tail`,
/// so the cost per chunk stays flat; [`trim_to_tail`] finishes the cut.
fn keep_tail(buffer: &mut Vec<u8>, bytes: &[u8], tail: Option<usize>) {
    buffer.extend_from_slice(bytes);
    if let Some(tail) = tail
        && buffer.len() >= tail.saturating_mul(2).max(1)
    {
        trim_to_tail(buffer, tail);
    }
}

fn trim_to_tail(buffer: &mut Vec<u8>, tail: usize) {
    let excess = buffer.len().saturating_sub(tail);
    buffer.drain(..excess);
}

impl Drop for Readers {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        for handle in self.handles.drain(..) {
            let _ = handle.join();
        }
    }
}

/// Read both pipes to end-of-file by `deadline`, blocking on the readers'
/// channel rather than polling; `None` when a pipe is still open at the
/// deadline, in which case its reader is cancelled and joined before this
/// returns. A read failure is the error itself, not an empty buffer.
fn drain_by<O, E>(
    stdout: Option<O>,
    stderr: Option<E>,
    deadline: Instant,
    limits: Limits,
) -> std::io::Result<Option<(Vec<u8>, Vec<u8>)>>
where
    O: Read + Pollable + Send + 'static,
    E: Read + Pollable + Send + 'static,
{
    let (report, results) = mpsc::channel();
    let mut readers = Readers {
        cancel: Arc::new(AtomicBool::new(false)),
        handles: Vec::new(),
    };
    let mut pending = 0;
    if let Some(reader) = stdout {
        readers.spawn(Pipe::Stdout, reader, report.clone(), limits);
        pending += 1;
    }
    if let Some(reader) = stderr {
        readers.spawn(Pipe::Stderr, reader, report.clone(), limits);
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
        BoundedError, Limits, POLL_CEILING, POLL_FLOOR, describe, drain_by, pause_schedule,
        run_bounded, spawn_bounded,
    };

    use super::Pollable;

    impl Pollable for &[u8] {
        fn readable_within(&self, _: Duration) -> std::io::Result<bool> {
            Ok(true)
        }
    }

    impl Pollable for std::io::Empty {
        fn readable_within(&self, _: Duration) -> std::io::Result<bool> {
            Ok(true)
        }
    }

    /// A pipe whose every read fails, as a broken pipe reader does.
    struct FailingRead;

    impl Pollable for FailingRead {
        fn readable_within(&self, _: Duration) -> std::io::Result<bool> {
            Ok(true)
        }
    }

    impl std::io::Read for FailingRead {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("pipe read failed"))
        }
    }

    #[test]
    fn a_failed_pipe_read_is_reported_not_emptied() {
        let deadline = Instant::now() + Duration::from_secs(5);
        let error = drain_by(
            Some(FailingRead),
            Some(std::io::empty()),
            deadline,
            Limits::default(),
        )
        .expect_err("the read failed");
        assert_eq!(error.to_string(), "pipe read failed");
        let error = BoundedError::from(error);
        assert!(matches!(error, BoundedError::Io(_)), "{error}");
    }

    #[test]
    fn drains_both_pipes_in_either_order() {
        let deadline = Instant::now() + Duration::from_secs(5);
        let drained = drain_by(
            Some(&b"out"[..]),
            Some(&b"err"[..]),
            deadline,
            Limits::default(),
        )
        .expect("reads succeed")
        .expect("both pipes close");
        assert_eq!(drained, (b"out".to_vec(), b"err".to_vec()));
        let drained = drain_by(
            None::<&[u8]>,
            Some(&b"err"[..]),
            deadline,
            Limits::default(),
        )
        .expect("reads succeed")
        .expect("the one pipe closes");
        assert_eq!(drained, (Vec::new(), b"err".to_vec()));
    }

    #[test]
    fn a_pipe_held_open_past_the_deadline_is_none() {
        // A reader that never becomes readable stands in for a descendant that
        // inherited the pipe; it only ends when it is cancelled.
        struct Held;
        impl std::io::Read for Held {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                unreachable!("never readable")
            }
        }
        impl Pollable for Held {
            fn readable_within(&self, wait: Duration) -> std::io::Result<bool> {
                std::thread::sleep(wait);
                Ok(false)
            }
        }
        let started = Instant::now();
        let deadline = started + Duration::from_millis(200);
        let drained = drain_by(
            Some(Held),
            Some(std::io::empty()),
            deadline,
            Limits::default(),
        )
        .expect("no read error");
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
    fn a_timeout_leaves_no_reader_thread_behind() {
        fn threads() -> usize {
            std::fs::read_dir("/proc/self/task").map_or(0, Iterator::count)
        }
        let before = threads();
        for _ in 0..4 {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 2 & wait"]);
            let error = run_bounded(command, Duration::from_millis(30)).expect_err("times out");
            assert!(matches!(error, BoundedError::TimedOut { .. }), "{error}");
        }
        // Other tests spawn threads in parallel, so only growth by the four
        // calls' own readers (8) would show a leak beyond this slack.
        assert!(
            threads() < before + 8,
            "{} threads from {before}",
            threads()
        );
    }

    #[test]
    fn a_cancelled_read_keeps_what_the_pipe_already_holds() {
        use std::sync::atomic::AtomicBool;
        let mut command = Command::new("sh");
        command.args(["-c", "printf held; sleep 2 & exit 0"]);
        let mut child = spawn_bounded(command, Duration::from_secs(5)).expect("sh spawns");
        let mut stdout = child.take_stdout().expect("piped");
        std::thread::sleep(Duration::from_millis(200));
        let cancel = AtomicBool::new(true);
        let mut kept = Vec::new();
        super::read_until_cancelled(&mut stdout, &cancel, |bytes| kept.extend_from_slice(bytes))
            .expect("reads");
        assert_eq!(kept, b"held");
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

    // Requirement: adapter-contract/Adapter output is bounded
    #[test]
    fn stdout_over_the_limit_is_an_error_and_the_child_is_reaped() {
        let child = spawn_bounded(Command::new("yes"), Duration::from_secs(30))
            .expect("yes spawns")
            .output_limit(1 << 20);
        let pid = child.id();
        let started = Instant::now();
        let error = child
            .wait_output()
            .expect_err("the output is over the limit");
        assert!(started.elapsed() < Duration::from_secs(10));
        match &error {
            BoundedError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::FileTooLarge),
            other => panic!("expected an io error, got {other}"),
        }
        assert!(process_is_gone(pid));
    }

    // Requirement: adapter-contract/Adapter output is bounded
    #[test]
    fn stdout_exactly_at_the_limit_is_kept() {
        let run = |limit: usize| {
            let mut command = Command::new("sh");
            command.args(["-c", "head -c 1000 /dev/zero"]);
            spawn_bounded(command, Duration::from_secs(10))
                .expect("sh spawns")
                .output_limit(limit)
                .wait_output()
        };
        let output = run(1000).expect("exactly the limit is within it");
        assert_eq!(output.stdout.len(), 1000);
        assert!(run(999).is_err(), "one byte over is an error, not a cut");
    }

    struct EndlessPipe;

    impl Pollable for EndlessPipe {
        fn readable_within(&self, _: Duration) -> std::io::Result<bool> {
            Ok(true)
        }
    }

    impl std::io::Read for EndlessPipe {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            buffer.fill(b'y');
            Ok(buffer.len())
        }
    }

    // Requirement: adapter-contract/Adapter output is bounded
    #[test]
    fn a_cancelled_read_of_a_writer_that_never_pauses_ends() {
        use std::sync::atomic::AtomicBool;
        let cancel = AtomicBool::new(true);
        let mut kept = 0usize;
        super::read_until_cancelled(&mut EndlessPipe, &cancel, |bytes| kept += bytes.len())
            .unwrap();
        assert!(kept >= super::CANCELLED_DRAIN_BYTES, "{kept}");
        assert!(kept < super::CANCELLED_DRAIN_BYTES + 8192, "{kept}");
    }

    // Requirement: adapter-contract/Adapter output is bounded
    #[test]
    fn stderr_keeps_the_tail_not_the_head() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf HEAD >&2; head -c 3145728 /dev/zero >&2; printf TAIL >&2; exit 3",
        ]);
        let output = spawn_bounded(command, Duration::from_secs(30))
            .expect("sh spawns")
            .stderr_tail(64 * 1024)
            .wait_output()
            .expect("stderr never fails the run");
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(output.stderr.len(), 64 * 1024);
        assert!(output.stderr.ends_with(b"TAIL"));
        assert!(!output.stderr.starts_with(b"HEAD"));
    }
}
