//! The runtime image must be able to run every tool the daemon spawns.
//!
//! # The failure this exists to stop
//!
//! `birdnet-behavior` does not capture audio in-process. It shells out:
//! `arecord` for every ALSA microphone, `ffmpeg` for RTSP, `PipeWire` and
//! Listen → Live, `ffmpeg`/`sox` for clip conversion, `kill` for the admin
//! Restart button. `install.sh` installs those packages and its own comment
//! (lines 716-754) records why it had to learn that lesson:
//!
//! > Only ffmpeg used to be ensured here, on the reasoning that "an ALSA
//! > microphone needs no ffmpeg" — [Raspberry Pi OS] ships alsa-utils so the
//! > gap stayed invisible; **on a minimal Debian it produces** [the failure].
//!
//! The `Dockerfile`'s runtime stage starts from `debian:*-slim`, which is a
//! minimal Debian, and for a long time it installed none of them. A container
//! built that way starts, serves a complete dashboard, passes its own
//! `HEALTHCHECK` (which only asks whether `SQLite` opens) and records nothing —
//! while `docker-compose.alsa.yml`, a shipped overlay whose entire purpose is
//! USB microphone capture, tells the operator this is the supported path.
//!
//! # Why a source-level gate and not only a container one
//!
//! `.github/workflows/docker.yml` also asserts this against the built image,
//! which is the real check. But that job needs a Docker daemon and a full
//! image build, so it does not run for a contributor adding a `Command::new`
//! in an editor. This test is static — it reads the `Dockerfile` and the call
//! sites — so it runs in the ordinary `cargo test` gate and fails the moment a
//! new external tool appears without anyone deciding whether the container
//! needs it.
//!
//! Adding a `Command::new("newtool")` therefore fails this test until
//! `newtool` is classified in [`TOOLS`] below. That is the point: the
//! classification is the decision, and it should be made deliberately.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Where a tool has to come from for the *container* to be able to run it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Provenance {
    /// Shipped by `debian:*-slim` itself (Priority: required / important
    /// packages that `debootstrap` installs). Nothing to add.
    DebianBase,
    /// Must be installed by the runtime stage's `apt-get install`, naming the
    /// package that provides it.
    Package(&'static str),
    /// Deliberately absent from the container: the feature it serves is either
    /// host-only or an optional integration the operator installs themselves.
    /// The reason is carried so the next reader does not have to reconstruct it.
    NotInContainer(&'static str),
}

/// Every external binary the non-test code spawns, and where the container is
/// expected to get it.
///
/// Kept sorted by tool name so a new entry lands somewhere predictable.
const TOOLS: &[(&str, Provenance)] = &[
    (
        "apprise",
        Provenance::NotInContainer(
            "optional notification backend; a Python package the operator installs, \
             and the integration degrades to 'not configured' without it",
        ),
    ),
    // Capture. The three that matter.
    ("arecord", Provenance::Package("alsa-utils")),
    ("df", Provenance::DebianBase),
    ("ffmpeg", Provenance::Package("ffmpeg")),
    ("getconf", Provenance::DebianBase), // libc-bin
    // `kill(1)`. The admin Restart button SIGTERMs its own pid and relies on
    // the supervisor restarting it; without procps it silently does nothing,
    // because the call site discards the result.
    ("kill", Provenance::Package("procps")),
    (
        "mount",
        Provenance::NotInContainer(
            "the tmpfs stream mount is a host-side concern; the container gets its \
             scratch space from the image's own /tmp",
        ),
    ),
    (
        "pactl",
        Provenance::NotInContainer(
            "PulseAudio/PipeWire probing is a desktop-host path; the container is \
             given /dev/snd directly",
        ),
    ),
    ("sox", Provenance::Package("sox")),
    (
        "systemctl",
        Provenance::NotInContainer("there is no systemd inside the container"),
    ),
    (
        "sftp",
        // The offsite backup target drives OpenSSH's own client rather than
        // carrying an in-process SSH stack: it is a large dependency and a
        // second place for key handling, host-key policy and cipher selection
        // to be subtly wrong. That trade only holds if the binary is actually
        // present, so the container installs it.
        Provenance::Package("openssh-client"),
    ),
    ("tar", Provenance::DebianBase),
    (
        "umount",
        Provenance::NotInContainer("the counterpart to `mount` — host-side tmpfs teardown"),
    ),
];

/// Repository root, from `CARGO_MANIFEST_DIR` (this test lives in the root
/// crate, so the manifest dir *is* the repo root).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `crates/` and `src/`, excluding each crate's own
/// `tests/` and `benches/` trees.
fn source_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for top in ["crates", "src"] {
        walk(&repo_root().join(top), &mut out);
    }
    out.sort();
    out
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // Integration tests, benchmarks and examples spawn things a shipped
            // container never runs (`sleep` in a capture-process fixture, for
            // one). Only production code is in scope.
            if matches!(&*name, "tests" | "benches" | "examples" | "target") {
                continue;
            }
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The part of `src` that is compiled into the binary — everything before the
/// first `#[cfg(test)]`.
///
/// Unit tests in this workspace live at the bottom of their file, so the first
/// `#[cfg(test)]` is the boundary. That is checked, not assumed: the
/// `no_command_spawn_hides_below_a_cfg_test` test below re-scans the tail and
/// requires every tool it finds there to be classified too, so a spawn placed
/// after the marker cannot slip through unnoticed either way.
fn production_half(src: &str) -> &str {
    src.split_once("#[cfg(test)]").map_or(src, |(head, _)| head)
}

/// Tools spawned in `src`, as `Command::new("…")` literals.
fn spawned_tools(src: &str) -> BTreeSet<String> {
    /// The literal form: `Command::new("ffmpeg")`.
    const LITERAL: &str = "Command::new(\"";
    /// The named form: `Command::new(SFTP_BINARY)`.
    const NAMED: &str = "Command::new(";

    let mut out = BTreeSet::new();

    let mut rest = src;
    while let Some(idx) = rest.find(LITERAL) {
        rest = &rest[idx + LITERAL.len()..];
        if let Some(end) = rest.find('"') {
            out.insert(rest[..end].to_owned());
        }
    }

    // The named form, where the same file declares
    // `const SFTP_BINARY: &str = "sftp";`.
    //
    // This was added because it had to be. The offsite SFTP target names its
    // binary once, in a constant the doctor check and the error messages share,
    // and the literal scanner above did not see it: removing `sftp` from
    // `TOOLS` entirely left this whole file green. A gate that only sees one
    // spelling of the thing it guards is a gate that stops working the first
    // time somebody tidies a string into a constant.
    let consts = string_consts(src);
    let mut rest = src;
    while let Some(idx) = rest.find(NAMED) {
        rest = &rest[idx + NAMED.len()..];
        let Some(end) = rest.find(')') else { continue };
        let arg = rest[..end].trim().trim_start_matches('&');
        if let Some(value) = consts.get(arg) {
            out.insert(value.clone());
        }
    }

    out
}

/// `const NAME: &str = "value";` declarations in one file.
fn string_consts(src: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for line in src.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("const ").or_else(|| {
            line.strip_prefix("pub const ")
                .or_else(|| line.strip_prefix("pub(crate) const "))
                .or_else(|| line.strip_prefix("pub(super) const "))
        }) else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(':') else {
            continue;
        };
        if !tail.trim_start().starts_with("&str") {
            continue;
        }
        let Some(after_eq) = tail.split_once('=').map(|(_, v)| v.trim()) else {
            continue;
        };
        let value = after_eq.trim_start_matches('"');
        if let Some(end) = value.find('"') {
            out.insert(name.trim().to_owned(), value[..end].to_owned());
        }
    }
    out
}

#[test]
fn the_scanner_sees_both_spellings_of_a_spawn() {
    // The scanner is what makes every other assertion in this file mean
    // something, so it is checked directly rather than trusted. It missed the
    // constant form once, and the file stayed green with `sftp` unclassified.
    let src = r#"
        pub const SFTP_BINARY: &str = "sftp";
        const OTHER: &str = "not-spawned";
        fn a() { Command::new("ffmpeg").arg("-i"); }
        fn b() { let mut c = Command::new(SFTP_BINARY); }
    "#;
    let found = spawned_tools(src);
    assert!(found.contains("ffmpeg"), "literal form missed: {found:?}");
    assert!(found.contains("sftp"), "constant form missed: {found:?}");
    assert!(
        !found.contains("not-spawned"),
        "a constant that is never spawned must not be reported: {found:?}"
    );
}

/// The package list of the `Dockerfile`'s **runtime** stage.
///
/// Scoped to the runtime stage deliberately: the builder stage installs `cmake`,
/// `g++` and friends, and finding `ffmpeg` there would prove nothing about the
/// image that ships.
fn runtime_stage_packages() -> BTreeSet<String> {
    let dockerfile = std::fs::read_to_string(repo_root().join("Dockerfile"))
        .expect("Dockerfile is readable from the repo root");
    let runtime = dockerfile
        .split_once("AS runtime")
        .expect("Dockerfile has a stage named `runtime`")
        .1;

    let mut packages = BTreeSet::new();
    let mut in_install = false;
    for line in runtime.lines() {
        let trimmed = line.trim();
        if trimmed.contains("apt-get install") {
            in_install = true;
            continue;
        }
        if !in_install {
            continue;
        }
        // The install list is one package per continued line; anything that is
        // not a bare package name ends it (`&& groupadd …`, a blank line, the
        // next instruction).
        let candidate = trimmed.trim_end_matches('\\').trim();
        if candidate.is_empty()
            || candidate.starts_with("&&")
            || candidate.starts_with('#')
            || candidate.contains(' ')
        {
            in_install = false;
            continue;
        }
        packages.insert(candidate.to_owned());
    }
    packages
}

fn classify(tool: &str) -> Option<Provenance> {
    TOOLS.iter().find(|(t, _)| *t == tool).map(|(_, p)| *p)
}

#[test]
fn every_spawned_tool_is_classified() {
    let mut unclassified: Vec<String> = Vec::new();
    for file in source_files() {
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        for tool in spawned_tools(production_half(&src)) {
            if classify(&tool).is_none() {
                unclassified.push(format!("{tool}  ({})", file.display()));
            }
        }
    }
    unclassified.sort();
    unclassified.dedup();
    assert!(
        unclassified.is_empty(),
        "these external tools are spawned but not classified in TOOLS — decide \
         whether the container needs them and add an entry:\n  {}",
        unclassified.join("\n  ")
    );
}

#[test]
fn no_command_spawn_hides_below_a_cfg_test() {
    // The production/test split above cuts at the first `#[cfg(test)]`. If a
    // real spawn ever lands below one, this catches it: everything in the tail
    // must be either classified or a known test-only helper.
    const TEST_ONLY: &[&str] = &["sleep"];
    let mut stray: Vec<String> = Vec::new();
    for file in source_files() {
        let Ok(src) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Some((_, tail)) = src.split_once("#[cfg(test)]") else {
            continue;
        };
        for tool in spawned_tools(tail) {
            if classify(&tool).is_none() && !TEST_ONLY.contains(&tool.as_str()) {
                stray.push(format!("{tool}  ({})", file.display()));
            }
        }
    }
    stray.sort();
    stray.dedup();
    assert!(
        stray.is_empty(),
        "spawned below a #[cfg(test)] and neither classified nor a known \
         test-only helper:\n  {}",
        stray.join("\n  ")
    );
}

#[test]
fn the_runtime_image_installs_every_tool_the_container_must_run() {
    let installed = runtime_stage_packages();
    assert!(
        installed.contains("ca-certificates"),
        "parsed the wrong block — the runtime stage installs ca-certificates, \
         and this test found: {installed:?}"
    );

    let mut missing: Vec<String> = Vec::new();
    for (tool, provenance) in TOOLS {
        if let Provenance::Package(pkg) = provenance
            && !installed.contains(*pkg)
        {
            missing.push(format!(
                "{tool} needs `{pkg}`, which the Dockerfile's runtime stage does not install"
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "the runtime image cannot run tools the daemon spawns:\n  {}\n\n\
         installed: {installed:?}",
        missing.join("\n  ")
    );
}

#[test]
fn the_alsa_overlay_is_only_shipped_if_the_image_can_use_it() {
    // docker-compose.alsa.yml exists solely to pass /dev/snd into the
    // container. Shipping it while the image has no `arecord` is the specific
    // combination that made the failure look supported.
    let overlay = repo_root().join("docker-compose.alsa.yml");
    if !overlay.exists() {
        return;
    }
    let installed = runtime_stage_packages();
    assert!(
        installed.contains("alsa-utils"),
        "docker-compose.alsa.yml is shipped as the supported USB-microphone \
         path, but the runtime image installs no alsa-utils, so `arecord` \
         cannot be spawned and the container records nothing"
    );
}
