//! Child processes that end with the panel.
//!
//! `PR_SET_PDEATHSIG` signals the child when the *thread* that forked it
//! exits, not the process. A stream is started on a `spawn_work` worker,
//! which exits as soon as the start returns, so a child forked there was
//! sent SIGTERM a moment after it started. Every child here is forked by one
//! thread that lives as long as the process, so the signal comes when the
//! panel dies (a crash or SIGKILL included) and not before.

use std::process::{Child, Command};
use std::sync::Mutex;
use std::sync::mpsc::{Sender, channel};

type Request = (Command, Sender<std::io::Result<Child>>);

static SPAWNER: Mutex<Option<Sender<Request>>> = Mutex::new(None);

/// Spawn `cmd` from the spawner thread, with SIGTERM on the panel's death.
pub fn spawn(mut cmd: Command) -> std::io::Result<Child> {
    die_with_parent(&mut cmd);
    let (reply, answer) = channel();
    let tx = {
        let mut slot = SPAWNER.lock().unwrap_or_else(|e| e.into_inner());
        slot.get_or_insert_with(start).clone()
    };
    tx.send((cmd, reply))
        .map_err(|_| std::io::Error::other("spawner thread is gone"))?;
    answer
        .recv()
        .map_err(|_| std::io::Error::other("spawner thread is gone"))?
}

fn start() -> Sender<Request> {
    let (tx, rx) = channel::<Request>();
    let spawned = std::thread::Builder::new()
        .name("swaypplet-children".into())
        .spawn(move || {
            for (mut cmd, reply) in rx {
                let _ = reply.send(cmd.spawn());
            }
        });
    if let Err(e) = spawned {
        log::warn!("cast: spawner thread: {e}");
    }
    tx
}

fn die_with_parent(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: prctl is async-signal-safe and touches only the child.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
}

/// End `child` and reap it. SIGTERM first, so wf-recorder and ffmpeg can
/// close their output; SIGKILL after `grace`.
pub fn end(child: &mut Child, grace: std::time::Duration) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    // SAFETY: kill(2) on our own child's pid, which `wait` has not reaped.
    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }
    let until = std::time::Instant::now() + grace;
    while std::time::Instant::now() < until {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;
    use std::time::Duration;

    fn sleeper() -> Command {
        let mut c = Command::new("sleep");
        c.arg("30");
        c
    }

    #[test]
    fn a_child_outlives_the_worker_that_asked_for_it() {
        let mut child = std::thread::spawn(|| spawn(sleeper()).unwrap())
            .join()
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert!(matches!(child.try_wait(), Ok(None)), "still running");
        end(&mut child, Duration::from_secs(2));
        assert!(child.try_wait().unwrap().is_some());
    }

    #[test]
    fn forked_from_a_worker_directly_it_would_die_with_the_worker() {
        // Why `spawn` exists: the same signal, set from a thread that ends.
        let mut child = std::thread::spawn(|| {
            let mut c = sleeper();
            die_with_parent(&mut c);
            c.spawn().unwrap()
        })
        .join()
        .unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.signal(), Some(libc::SIGTERM));
    }
}
