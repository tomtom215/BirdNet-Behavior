//! A login session survives a restart of the station (DD-15).
//!
//! `/station/access` promised "Sessions last up to 14 days". On every install
//! the bare-metal installer produces, they lasted until the next restart: the
//! signing secret was read from `BNB_SESSION_SECRET` or `CADDY_PWD` in the
//! *environment*, the installer writes `CADDY_PWD` to the config file, nothing
//! exports it, and the unit sets neither variable — so each process minted a
//! random secret and every cookie from the previous one failed to verify.
//!
//! The station now keeps a secret it generated once beside its database. This
//! boots the real binary twice on one config with no environment, signs in on
//! the first process, and presents that cookie to the second.

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_birdnet-behavior");
const PASSWORD: &str = "restart-probe-password";

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// One request over a fresh connection: status line and headers, body.
fn http(port: u16, request: &str) -> Option<(String, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    Some((
        head,
        String::from_utf8_lossy(&buf[split + 4..]).into_owned(),
    ))
}

fn get(port: u16, path: &str, cookie: Option<&str>) -> Option<String> {
    let cookie_line = cookie.map_or(String::new(), |c| format!("Cookie: {c}\r\n"));
    http(
        port,
        &format!(
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n{cookie_line}Connection: close\r\n\r\n"
        ),
    )
    .map(|(head, _)| head)
}

fn boot(config_path: &std::path::Path, port: u16) -> ChildGuard {
    let child = Command::new(BIN)
        .args([
            "--web-only",
            "--config",
            config_path.to_str().unwrap(),
            "--listen",
            &format!("127.0.0.1:{port}"),
        ])
        .env("RUST_LOG", "warn")
        .env_remove("BIRDNET_CONFIG")
        // The environment the installer's unit gives the service: none of the
        // three variables the old derivation could read.
        .env_remove("BNB_SESSION_SECRET")
        .env_remove("CADDY_PWD")
        .env_remove("CADDY_USER")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {BIN}: {e}"));
    let mut guard = ChildGuard(child);
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if let Some(status) = guard.0.try_wait().expect("try_wait") {
            panic!("server exited during startup with {status}");
        }
        if let Some(head) = get(port, "/api/v2/health", None)
            && head.starts_with("HTTP/1.1 200")
        {
            return guard;
        }
        assert!(
            Instant::now() < deadline,
            "server did not come up within 45s"
        );
        std::thread::sleep(Duration::from_millis(300));
    }
}

fn status_of(head: &str) -> &str {
    head.lines().next().unwrap_or_default()
}

fn header_of<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

#[test]
fn a_session_from_the_first_boot_still_verifies_on_the_second() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config_path = dir.path().join("birdnet.conf");
    // `CADDY_PWD` in the *config file*, as the installer writes it: the old
    // derivation could not see it there.
    std::fs::write(
        &config_path,
        format!(
            "SITENAME=Restart Probe\nDB_PATH={}\nCADDY_PWD={PASSWORD}\n",
            dir.path().join("birds.db").display()
        ),
    )
    .unwrap();
    let port = free_port();

    let cookie = {
        let _first = boot(&config_path, port);
        let body = format!("username=admin&password={PASSWORD}");
        let (head, _) = http(
            port,
            &format!(
                "POST /login HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://127.0.0.1\r\n\
                 Content-Type: application/x-www-form-urlencoded\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        )
        .expect("login answers");
        assert!(status_of(&head).contains("303"), "login: {head}");
        let set_cookie = header_of(&head, "set-cookie").expect("a session cookie");
        let pair = set_cookie.split(';').next().unwrap().to_owned();
        assert!(pair.starts_with("bnb-session="), "{pair}");

        let head = get(port, "/admin/overview", Some(&pair)).expect("admin page");
        assert!(
            status_of(&head).contains("200"),
            "signed in on boot 1: {head}"
        );
        pair
        // `_first` is killed here.
    };

    let _second = boot(&config_path, port);
    let head = get(port, "/admin/overview", Some(&cookie)).expect("admin page");
    assert!(
        status_of(&head).contains("200"),
        "the cookie from boot 1 must still verify on boot 2: {}",
        status_of(&head)
    );
    let secret_file = dir.path().join(birdnet_web::session::SECRET_FILE_NAME);
    assert!(
        secret_file.is_file(),
        "the first boot persisted a secret beside the database"
    );
    // The control: no cookie at all is still a redirect to sign in, so the
    // 200 above is the session and not an open station.
    let head = get(port, "/admin/overview", None).expect("admin page");
    assert!(
        status_of(&head).contains("303"),
        "no cookie: {}",
        status_of(&head)
    );
}
