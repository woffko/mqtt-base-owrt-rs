// SPDX-License-Identifier: GPL-2.0-or-later
//! Run a helper program with a deadline and a bounded output buffer.
//!
//! Collectors call `ubus`, `uci`, `uname` and similar tools. A hung or chatty
//! helper must never stall the publish loop or grow memory without bound.

use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Output {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub truncated: bool,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

const POLL_STEP: Duration = Duration::from_millis(10);
/// How long the output may take to end after the child has exited. A process
/// the child left behind can keep the pipe open.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);

/// Run `program args`, feed `stdin` if given, and collect at most
/// `max_output` bytes of stdout. Stderr is discarded. The child is killed when
/// `timeout` expires and `ErrorKind::TimedOut` is returned.
pub fn run(
    program: &str,
    args: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
    max_output: usize,
) -> io::Result<Output> {
    run_with_grace(program, args, stdin, timeout, max_output, OUTPUT_GRACE)
}

fn run_with_grace(
    program: &str,
    args: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
    max_output: usize,
    grace: Duration,
) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    if let (Some(data), Some(mut pipe)) = (stdin, child.stdin.take()) {
        // Inputs are small JSON documents; a broken pipe only means the
        // helper did not read them, which shows up in its exit status.
        let _ = pipe.write_all(data);
    }

    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::other("stdout pipe missing"));
    };

    let deadline = Instant::now() + timeout;
    let (sender, receiver) = mpsc::channel();
    // A thread that cannot be started is an error, not a panic (the release
    // build aborts on panic), and it must not leave the child running.
    let spawned = thread::Builder::new()
        .name("exec-reader".to_string())
        .spawn(move || {
            let mut buf = Vec::with_capacity(max_output.min(64 * 1024));
            let mut chunk = [0u8; 8192];
            let mut truncated = false;
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        let room = max_output.saturating_sub(buf.len());
                        if n > room {
                            truncated = true;
                        }
                        buf.extend_from_slice(&chunk[..n.min(room)]);
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = sender.send((buf, truncated));
        });
    if let Err(e) = spawned {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }

    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => thread::sleep(POLL_STEP),
        }
    };
    match status {
        Some(status) => {
            let (stdout, truncated) = receiver
                .recv_timeout(grace)
                .map_err(|_| io::Error::other("the output of the helper did not end"))?;
            Ok(Output {
                code: status.code(),
                stdout,
                truncated,
            })
        }
        None => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("{program} did not finish within {} s", timeout.as_secs()),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_output_and_exit_code() {
        let out = run(
            "sh",
            &["-c", "echo hello; exit 3"],
            None,
            Duration::from_secs(5),
            1024,
        )
        .unwrap();
        assert_eq!(out.code, Some(3));
        assert_eq!(out.stdout, b"hello\n");
        assert!(!out.truncated);
    }

    #[test]
    fn feeds_stdin() {
        let out = run("cat", &[], Some(b"{\"a\":1}"), Duration::from_secs(5), 1024).unwrap();
        assert!(out.success());
        assert_eq!(out.stdout, b"{\"a\":1}");
    }

    #[test]
    fn truncates_large_output() {
        let out = run(
            "sh",
            &["-c", "head -c 100000 /dev/zero"],
            None,
            Duration::from_secs(5),
            1000,
        )
        .unwrap();
        assert_eq!(out.stdout.len(), 1000);
        assert!(out.truncated);
    }

    #[test]
    fn a_process_left_behind_does_not_hold_the_caller() {
        // The shell exits at once; the background sleep keeps the pipe open.
        let started = Instant::now();
        let err = run_with_grace(
            "sh",
            &["-c", "sleep 5 & echo hi"],
            None,
            Duration::from_secs(5),
            1024,
            Duration::from_millis(200),
        )
        .unwrap_err();
        assert!(err.to_string().contains("did not end"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn kills_on_timeout() {
        let started = Instant::now();
        let err = run("sleep", &["5"], None, Duration::from_millis(200), 1024).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}
