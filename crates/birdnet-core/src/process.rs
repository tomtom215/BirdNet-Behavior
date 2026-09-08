//! Child processes that cannot hang the station (PR-8).
//!
//! Every tool the station shells out to — `ffmpeg`, `sox`, `tar`, `df`,
//! `arecord`, `mount`, `timedatectl`, `systemctl` — is waited on synchronously
//! somewhere, and several of those waits sit on the single event-processor
//! thread or in a periodic health probe. `std::process` offers no bounded
//! wait, so one child that never exits (a `df` on a dead network mount, an
//! `ffmpeg` stuck on a device that stopped answering) stopped that thread for
//! ever, the event channel filled, the heartbeat froze, and the watchdog
//! restarted the station without a line saying why.
//!
//! [`run_with_timeout`] is the one way a synchronous wait is allowed to
//! happen. It drains both pipes on their own threads (so a chatty child cannot
//! deadlock against a full pipe), polls the child until the deadline, and on
//! the deadline kills it, reaps it, and answers
//! [`std::io::ErrorKind::TimedOut`] naming the program and the limit.
//! `tests/every_child_process_has_a_deadline.rs` keeps every other spawn from
//! waiting on its own.

use std::io::{self, Read};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// How often the child is polled for exit while it runs. The first poll is
/// immediate, so a child that exits at once costs one `try_wait`.
const POLL: Duration = Duration::from_millis(10);

/// Run `cmd` to completion and return its output, or kill it at `timeout`.
///
/// Standard input is closed; standard output and error are captured, whatever
/// the caller set. A child that exits within the limit gives `Ok(Output)` with
/// its exit status, as `Command::output` would. One that has not exited by
/// the limit is sent `SIGKILL`, reaped, and reported as an error of kind
/// [`io::ErrorKind::TimedOut`] whose message names the program and the limit.
///
/// A grandchild the killed child left behind can keep a pipe open; the reader
/// threads are then left to finish on their own rather than joined, so the
/// caller gets its answer at the deadline either way.
///
/// # Errors
///
/// The spawn error when the program cannot be started (typically
/// [`io::ErrorKind::NotFound`]), any error from waiting on it, or
/// [`io::ErrorKind::TimedOut`] when it outlives `timeout`.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> io::Result<Output> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || drain(stdout));
    let stderr_reader = thread::spawn(move || drain(stderr));

    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            // Kill and reap; a child already gone between the poll and the kill
            // makes `kill` an error worth ignoring, and `wait` still reaps it.
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "{program} did not exit within {} s and was killed",
                    timeout.as_secs_f64()
                ),
            ));
        }
        thread::sleep(POLL);
    };

    // The child has exited, so both pipes will reach end-of-file once any
    // grandchild that inherited them is gone too; for the ordinary case that
    // is immediate.
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

/// Read a pipe to end-of-file. A read error ends the capture with what was
/// read so far; the exit status is the verdict, not the pipe.
fn drain<R: Read>(pipe: Option<R>) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Some(mut pipe) = pipe {
        let _ = pipe.read_to_end(&mut buf);
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate for PR-8: a child that never exits does not hold the caller.
    ///
    /// `sh -c 'echo $$ > pidfile; exec sleep 30'` puts the sleep under the
    /// pid it wrote, so the test can check the process is gone afterwards
    /// rather than take the error's word for it.
    #[test]
    fn a_child_that_outlives_its_deadline_is_killed_and_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("pid");
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("echo $$ > '{}'; exec sleep 30", pidfile.display()));

        let started = Instant::now();
        let err = run_with_timeout(&mut cmd, Duration::from_millis(300))
            .expect_err("a 30 s sleep must not complete inside 300 ms");
        let elapsed = started.elapsed();

        assert_eq!(err.kind(), io::ErrorKind::TimedOut, "{err}");
        assert!(
            err.to_string().contains("sh did not exit within 0.3 s"),
            "the error must name the program and the limit: {err}"
        );
        assert!(
            elapsed < Duration::from_secs(5),
            "the caller waited {elapsed:?} for a 300 ms deadline"
        );

        let pid: u32 = std::fs::read_to_string(&pidfile)
            .expect("the shell wrote its pid before exec")
            .trim()
            .parse()
            .expect("a pid");
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "pid {pid} is still alive (or a zombie) after the deadline"
        );
    }

    /// Counterpart: a child that exits in time is reported exactly as
    /// `Command::output` would report it, status and both pipes.
    #[test]
    fn a_child_that_exits_in_time_returns_its_status_and_output() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo out; echo err >&2; exit 3"]);
        let out = run_with_timeout(&mut cmd, Duration::from_secs(10)).expect("runs");
        assert_eq!(out.status.code(), Some(3));
        assert_eq!(out.stdout, b"out\n");
        assert_eq!(out.stderr, b"err\n");
    }

    /// A child that writes more than a pipe buffer on both streams must not
    /// deadlock against a reader that waits for exit before draining.
    #[test]
    fn a_chatty_child_is_drained_rather_than_deadlocked() {
        let mut cmd = Command::new("sh");
        cmd.args([
            "-c",
            "head -c 3000000 /dev/zero; head -c 3000000 /dev/zero >&2",
        ]);
        let started = Instant::now();
        let out = run_with_timeout(&mut cmd, Duration::from_secs(20)).expect("runs");
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 3_000_000);
        assert_eq!(out.stderr.len(), 3_000_000);
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "draining 6 MB took {:?}; a pipe was blocking",
            started.elapsed()
        );
    }

    #[test]
    fn a_missing_program_is_the_spawn_error() {
        let mut cmd = Command::new("/nonexistent/birdnet-no-such-tool");
        let err = run_with_timeout(&mut cmd, Duration::from_secs(1)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }
}
