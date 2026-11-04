//! Single instance and remote commands over a Unix socket in `$XDG_RUNTIME_DIR`: the first process
//! binds it; later ones connect and send a word (`toggle`, `options`) instead of starting.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::paths;

static LISTENER: OnceLock<Mutex<Option<UnixListener>>> = OnceLock::new();

/// `true` when this process now owns the socket (no live instance answered on it).
pub fn claim() -> bool {
    if LISTENER.get().is_some() {
        return true;
    }
    let path = paths::control_socket();
    if UnixStream::connect(&path).is_ok() {
        return false;
    }
    // Nobody answers: a stale file from a crash, or nothing at all.
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(l) => {
            let _ = LISTENER.set(Mutex::new(Some(l)));
            true
        }
        // Another process won the race between our connect and bind.
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => false,
        Err(e) => {
            // No usable runtime dir (or a path too long for a socket): run anyway, just without
            // single-instance and `--toggle`.
            log::warn!(
                "control socket {}: {e}; running without single-instance",
                path.display()
            );
            let _ = LISTENER.set(Mutex::new(None));
            true
        }
    }
}

/// Like [`claim`], retrying for `wait` (a freshly updated copy starts while the old one exits).
pub fn claim_waiting(wait: Duration) -> bool {
    let deadline = std::time::Instant::now() + wait;
    loop {
        if claim() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Send one command word to the running instance.
pub fn send(cmd: &str) -> std::io::Result<()> {
    let mut s = UnixStream::connect(paths::control_socket())?;
    s.set_write_timeout(Some(Duration::from_secs(2)))?;
    writeln!(s, "{cmd}")
}

/// Serve commands on a thread; each line received is handed to `on_cmd`.
pub fn serve(on_cmd: impl Fn(String) + Send + 'static) {
    let Some(listener) = LISTENER
        .get()
        .and_then(|m| m.lock().ok().and_then(|mut l| l.take()))
    else {
        return;
    };
    std::thread::Builder::new()
        .name("op36-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let word = line.trim().to_string();
                    if !word.is_empty() {
                        on_cmd(word);
                    }
                }
            }
        })
        .expect("spawn control thread");
}

/// Remove the socket file at exit so the next start does not have to probe it.
pub fn release() {
    let _ = std::fs::remove_file(paths::control_socket());
}
