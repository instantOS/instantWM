//! A single pending wallpaper request and an explicitly owned helper process.

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

struct WallpaperChild {
    child: Child,
    exit: OwnedFd,
}

impl WallpaperChild {
    fn spawn(command: &mut Command) -> io::Result<Self> {
        let mut child = command.spawn()?;
        // The unreaped child reserves its PID; the pidfd notifies us on exit.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, child.id(), 0) };
        if fd < 0 {
            let error = io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(Self {
            child,
            exit: unsafe { OwnedFd::from_raw_fd(fd as i32) },
        })
    }
}

impl Drop for WallpaperChild {
    fn drop(&mut self) {
        // Replacement and shutdown stop and reap only this worker's child.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub(crate) struct WallpaperProcess {
    worker: Option<Worker>,
}

#[derive(Default)]
struct Requests {
    latest: Option<Command>,
    stopped: bool,
}

struct Worker {
    requests: Arc<Mutex<Requests>>,
    wake: UnixStream,
    thread: JoinHandle<()>,
    finished: mpsc::Receiver<()>,
}

impl WallpaperProcess {
    /// Queue a replacement. Only the worker launches, stops, and reaps helpers.
    pub(crate) fn set(&mut self, path: &str) -> io::Result<()> {
        let mut command = Command::new("swaybg");
        command.args(["-i", path, "-m", "fill"]);
        self.launch(command)
    }

    fn launch(&mut self, command: Command) -> io::Result<()> {
        if self.worker.is_none() {
            let (wake, receiver) = UnixStream::pair()?;
            wake.set_nonblocking(true)?;
            receiver.set_nonblocking(true)?;
            let requests = Arc::new(Mutex::new(Requests::default()));
            let shared = requests.clone();
            let (finished_tx, finished) = mpsc::channel();
            let thread = std::thread::Builder::new()
                .name("instantwm-wallpaper".into())
                .spawn(move || {
                    if let Err(error) = run_worker(&shared, receiver) {
                        log::error!("wallpaper worker stopped: {error}");
                    }
                    let mut requests = shared.lock().unwrap();
                    requests.stopped = true;
                    requests.latest = None;
                    let _ = finished_tx.send(());
                })?;
            self.worker = Some(Worker {
                requests,
                wake,
                thread,
                finished,
            });
        }
        let worker = self.worker.as_mut().unwrap();
        {
            let mut requests = worker.requests.lock().unwrap();
            if requests.stopped {
                return Err(io::Error::other("wallpaper worker stopped"));
            }
            // Replacing an unstarted command uses constant space and never
            // launches an obsolete wallpaper merely to kill it again.
            requests.latest = Some(command);
        }
        notify(&mut worker.wake)
    }

    /// Call after input/display dispatch stops, before the runtime exits.
    pub(crate) fn shutdown(&mut self) {
        self.shutdown_with_timeout(Duration::from_secs(1));
    }

    fn shutdown_with_timeout(&mut self, timeout: Duration) {
        if let Some(mut worker) = self.worker.take() {
            {
                let mut requests = worker.requests.lock().unwrap();
                requests.stopped = true;
                requests.latest = None;
            }
            let _ = notify(&mut worker.wake);
            if matches!(
                worker.finished.recv_timeout(timeout),
                Err(mpsc::RecvTimeoutError::Timeout)
            ) {
                // A blocked exec handshake or reap cannot be cancelled through
                // std::process. Leave ownership with the worker so it can still
                // stop and reap the helper if the operation completes.
                log::warn!("wallpaper worker did not stop before the shutdown deadline");
                return;
            }
            if worker.thread.join().is_err() {
                log::error!("wallpaper worker panicked during shutdown");
            }
        }
    }
}

impl Drop for WallpaperProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn notify(wake: &mut UnixStream) -> io::Result<()> {
    loop {
        match wake.write(&[1]) {
            Ok(_) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn run_worker(requests: &Mutex<Requests>, mut wake: UnixStream) -> io::Result<()> {
    let mut current: Option<WallpaperChild> = None;
    loop {
        let mut fds = [
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: current.as_ref().map_or(-1, |child| child.exit.as_raw_fd()),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        // Reap an exited helper before processing a replacement request.
        if fds[1].revents != 0
            && let Some(mut child) = current.take()
        {
            let status = child.child.wait()?;
            if !status.success() {
                log::warn!("wallpaper helper exited with {status}");
            }
        }
        if fds[0].revents != 0 {
            let mut bytes = [0; 256];
            loop {
                match wake.read(&mut bytes) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error),
                }
            }
            let command = {
                let mut requests = requests.lock().unwrap();
                if requests.stopped {
                    return Ok(());
                }
                requests.latest.take()
            };
            if let Some(mut command) = command {
                match WallpaperChild::spawn(&mut command) {
                    Ok(child) => current = Some(child),
                    Err(error) => log::error!("failed to launch wallpaper: {error}"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn command(path: &Path) -> Command {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "echo $$ > \"$1\"; exec /bin/sleep 30",
                "wallpaper-test",
            ])
            .arg(path);
        command
    }

    fn pid(path: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(value) = std::fs::read_to_string(path)
                && let Ok(pid) = value.trim().parse()
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "helper did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_until_reaped(pid: u32) {
        let path = format!("/proc/{pid}");
        let deadline = Instant::now() + Duration::from_secs(5);
        while Path::new(&path).exists() {
            assert!(Instant::now() < deadline, "child {pid} was not reaped");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn replacement_and_shutdown_stop_only_owned_children() {
        let directory = tempfile::tempdir().unwrap();
        let mut unrelated = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let mut owner = WallpaperProcess::default();
        let first = directory.path().join("first");
        owner.launch(command(&first)).unwrap();
        let first_pid = pid(&first);
        owner
            .launch(Command::new("/nonexistent/instantwm-wallpaper-test"))
            .unwrap();
        let second = directory.path().join("second");
        owner.launch(command(&second)).unwrap();
        let second_pid = pid(&second);
        wait_until_reaped(first_pid);
        owner.shutdown();
        assert!(!Path::new(&format!("/proc/{second_pid}")).exists());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }

    #[test]
    fn exited_child_is_reaped_without_another_request() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pid");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "echo $$ > \"$1\"", "wallpaper-test"])
            .arg(&path);
        let mut owner = WallpaperProcess::default();
        owner.launch(command).unwrap();
        wait_until_reaped(pid(&path));
    }

    #[test]
    fn blocked_launch_does_not_block_requests_and_only_latest_pending_launches() {
        use std::os::unix::process::CommandExt;

        let directory = tempfile::tempdir().unwrap();
        let (mut release, gate) = UnixStream::pair().unwrap();
        gate.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let (signal, mut started) = UnixStream::pair().unwrap();
        started
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut first = command(&directory.path().join("first"));
        // Gate the exec handshake itself, not a running helper. Submission
        // must remain responsive even when Command::spawn cannot finish yet.
        unsafe {
            first.pre_exec(move || {
                let mut byte = 1u8;
                if libc::write(signal.as_raw_fd(), (&byte as *const u8).cast(), 1) != 1
                    || libc::read(gate.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) != 1
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut owner = WallpaperProcess::default();
        owner.launch(first).unwrap();
        started.read_exact(&mut [0]).unwrap();
        let obsolete = directory.path().join("obsolete");
        owner.launch(command(&obsolete)).unwrap();
        let latest = directory.path().join("latest");
        owner.launch(command(&latest)).unwrap();
        release.write_all(&[1]).unwrap();
        let latest_pid = pid(&latest);
        assert!(!obsolete.exists());
        owner.shutdown();
        assert!(!Path::new(&format!("/proc/{latest_pid}")).exists());
    }

    #[test]
    fn shutdown_does_not_wait_for_a_blocked_launch() {
        use std::os::unix::process::CommandExt;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pid");
        let (mut release, gate) = UnixStream::pair().unwrap();
        let (signal, mut started) = UnixStream::pair().unwrap();
        started
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut command = command(&path);
        unsafe {
            command.pre_exec(move || {
                let pid = libc::getpid().to_ne_bytes();
                let mut byte = 1u8;
                if libc::write(signal.as_raw_fd(), pid.as_ptr().cast(), pid.len())
                    != pid.len() as isize
                    || libc::read(gate.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) != 1
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut owner = WallpaperProcess::default();
        owner.launch(command).unwrap();
        let mut pid_bytes = [0; 4];
        started.read_exact(&mut pid_bytes).unwrap();
        let child_pid = i32::from_ne_bytes(pid_bytes) as u32;
        let before = Instant::now();
        owner.shutdown_with_timeout(Duration::from_millis(20));
        // Release before asserting so a failed assertion cannot strand the child.
        let elapsed = before.elapsed();
        release.write_all(&[1]).unwrap();
        assert!(elapsed < Duration::from_secs(1));
        assert!(owner.worker.is_none());
        wait_until_reaped(child_pid);
    }
}
