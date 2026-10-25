//! Restarting the process by running the same program again.
//!
//! Under systemd a restart is "exit, and `Restart=always` starts a new one". A
//! station that nobody supervises — the binary run from a terminal, a container
//! started without a restart policy — has no one to start the new one, so the
//! restart control used to refuse, and the setup wizard ended on "restart the
//! station" with no way for a browser to do it.
//!
//! This is the other half for those stations: the web layer says a restart is
//! wanted ([`request_reexec`]), the process shuts down exactly as it does for
//! SIGTERM (connections drained, detection loop stopped, database closed), and
//! once that has finished the binary's `main` replaces itself with a fresh copy
//! of the same program and arguments ([`reexec_command`]). Same process id, so a
//! container's PID 1 stays PID 1.
//!
//! It is **not** used under systemd: there the exit-and-restart path is the one
//! the unit's watchdog, `Type=notify` and sandbox were written for.

use std::sync::atomic::{AtomicBool, Ordering};

static REEXEC: AtomicBool = AtomicBool::new(false);

/// Ask for the process to run itself again once it has shut down.
pub fn request_reexec() {
    REEXEC.store(true, Ordering::SeqCst);
}

/// Take back a [`request_reexec`] whose SIGTERM was never sent.
///
/// The request has to be made *before* the signal (the shutdown it starts reads
/// the flag), so a signal that then fails to go out would leave the flag set for
/// the next, unrelated graceful shutdown to act on.
pub fn cancel_reexec() {
    REEXEC.store(false, Ordering::SeqCst);
}

/// Whether [`request_reexec`] has been called (and not taken back).
#[must_use]
pub fn reexec_requested() -> bool {
    REEXEC.load(Ordering::SeqCst)
}

/// The command that starts this program again with the arguments it was started
/// with.
///
/// # Errors
///
/// When the path of the running program cannot be read.
pub fn reexec_command() -> std::io::Result<std::process::Command> {
    let exe = std::env::current_exe()?;
    Ok(command_for(&exe, std::env::args_os().skip(1)))
}

/// [`reexec_command`] with the program and arguments given, for tests.
///
/// A program replaced on disk while it ran (an update) reports its path with a
/// ` (deleted)` suffix on Linux; the new file is at the path without it.
pub fn command_for<I>(exe: &std::path::Path, args: I) -> std::process::Command
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let text = exe.to_string_lossy();
    let path = text
        .strip_suffix(" (deleted)")
        .map_or_else(|| exe.to_path_buf(), std::path::PathBuf::from);
    let mut command = std::process::Command::new(path);
    command.args(args);
    command
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::path::Path;

    /// The path of an executable that was replaced while running names a file
    /// that no longer exists; the replacement is at the plain path.
    #[test]
    fn a_replaced_executable_is_started_from_its_plain_path() {
        let c = command_for(
            Path::new("/usr/local/bin/birdnet-behavior (deleted)"),
            [OsString::from("--config"), OsString::from("/etc/b.conf")],
        );
        assert_eq!(c.get_program(), "/usr/local/bin/birdnet-behavior");
        let args: Vec<_> = c.get_args().collect();
        assert_eq!(args, ["--config", "/etc/b.conf"]);
    }

    #[test]
    fn an_ordinary_path_is_kept_as_it_is() {
        let c = command_for(Path::new("/opt/bnb/birdnet-behavior"), []);
        assert_eq!(c.get_program(), "/opt/bnb/birdnet-behavior");
    }
}
