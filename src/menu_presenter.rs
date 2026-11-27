//! Event-driven instantMENU process lifecycle shared by WM-owned choosers.
use crate::contexts::WmCtx;
use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::OwnedFd;
use std::process::{Child, Command};

type WatchOutput<C> = dyn Fn(File, u64, C) -> Result<RegistrationToken, String>;

pub(crate) struct MenuHost<C> {
    watch: Option<Box<WatchOutput<C>>>,
    remove: Option<Box<dyn Fn(RegistrationToken)>>,
    pub(crate) source: Option<RegistrationToken>,
    pub(crate) generation: u64,
    pub(crate) child: Option<Child>,
}

impl<C> MenuHost<C> {
    pub(crate) fn is_installed(&self) -> bool {
        self.watch.is_some()
    }

    pub(crate) fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(token) = self.source.take()
            && let Some(remove) = &self.remove
        {
            remove(token);
        }
        if let Some(child) = self.child.take() {
            finish_child(child);
        }
    }
}

/// Terminate a presenter that an answer did not already close. instantMENU may
/// keep running after writing its selection; leaving it alive would linger over
/// the new focus until the compositor exits.
pub(crate) fn finish_child(mut child: Child) {
    // Keep the child unreaped until after signalling, so its pid cannot be
    // reused for an unrelated process group.
    if matches!(child.try_wait(), Ok(None)) {
        if let Ok(pgid) = i32::try_from(child.id()) {
            if !crate::util::signal_process_group(pgid, rustix::process::Signal::TERM) {
                let _ = child.kill();
            }
        } else {
            let _ = child.kill();
        }
    }
    crate::util::reap_child_async(child);
}

impl<C> Drop for MenuHost<C> {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl<C> Default for MenuHost<C> {
    fn default() -> Self {
        Self {
            watch: None,
            remove: None,
            source: None,
            generation: 0,
            child: None,
        }
    }
}

/// Backend wiring supplies only access to its shared context. Sources are
/// installed on demand and removed after a choice, dismissal, or cancellation.
pub(crate) fn install<T: 'static, C: 'static>(
    handle: LoopHandle<'static, T>,
    host: &mut MenuHost<C>,
    context: for<'a> fn(&'a mut T) -> WmCtx<'a>,
    finish: fn(&mut WmCtx<'_>, u64, C, &str),
) {
    let remove_handle = handle.clone();
    host.remove = Some(Box::new(move |token| remove_handle.remove(token)));
    host.watch = Some(Box::new(move |output, generation, menu_context| {
        let mut buffer = Vec::new();
        let mut menu_context = Some(menu_context);
        handle
            .insert_source(
                Generic::new(output, Interest::READ, Mode::Level),
                move |_, output, state| {
                    let answer = match read_answer(&mut &**output, &mut buffer) {
                        Ok(None) => return Ok(PostAction::Continue),
                        Ok(Some(answer)) => answer,
                        Err(error) => {
                            log::debug!("menu output: {error}");
                            String::new()
                        }
                    };
                    finish(
                        &mut context(state),
                        generation,
                        menu_context.take().expect("menu response is handled once"),
                        answer.trim_end(),
                    );
                    Ok(PostAction::Remove)
                },
            )
            .map_err(|error| format!("failed to watch menu: {error}"))
    }));
}

/// Read a bounded, nonblocking single-line response. Empty EOF is dismissal.
pub(crate) fn read_answer(
    output: &mut impl Read,
    buffer: &mut Vec<u8>,
) -> io::Result<Option<String>> {
    let mut chunk = [0; 128];
    loop {
        match output.read(&mut chunk) {
            Ok(0) => {
                return String::from_utf8(std::mem::take(buffer))
                    .map(Some)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
            }
            Ok(count) => {
                buffer.extend_from_slice(&chunk[..count]);
                if buffer.len() > 128 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "menu response too long",
                    ));
                }
                if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                    return String::from_utf8(buffer[..end].to_vec())
                        .map(Some)
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn start<C>(
    host: &mut MenuHost<C>,
    mut command: Command,
    context: C,
    input: String,
) -> Result<(), String> {
    if host.watch.is_none() {
        return Err("menu event source is not configured".into());
    }
    host.cancel();
    let child = command
        .spawn()
        .map_err(|error| format!("failed to open menu: {error}"))?;
    host.child = Some(child);
    let result = (|| {
        let child = host.child.as_mut().unwrap();
        let output = child.stdout.take().expect("piped stdout");
        let output = File::from(OwnedFd::from(output));
        // Preserve existing descriptor flags when enabling nonblocking reads.
        let flags = rustix::fs::fcntl_getfl(&output).map_err(|error| error.to_string())?;
        rustix::fs::fcntl_setfl(&output, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|error| error.to_string())?;
        // Window lists can exceed pipe capacity. Feed them off the event loop
        // so a slow or failed presenter cannot freeze the compositor.
        let mut stdin = child.stdin.take().expect("piped stdin");
        std::thread::Builder::new()
            .name("menu-input".into())
            .spawn(move || {
                if let Err(error) = stdin.write_all(input.as_bytes()) {
                    log::debug!("menu input: {error}");
                }
            })
            .map_err(|error| format!("failed to feed menu: {error}"))?;
        host.source = Some(host.watch.as_ref().unwrap()(
            output,
            host.generation,
            context,
        )?);
        Ok(())
    })();
    if result.is_err() {
        host.cancel();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    #[test]
    fn read_answer_waits_for_a_complete_line() {
        let (mut output, mut writer) = UnixStream::pair().unwrap();
        output.set_nonblocking(true).unwrap();
        let mut buffer = Vec::new();
        writer.write_all(b"win").unwrap();
        assert_eq!(read_answer(&mut output, &mut buffer).unwrap(), None);
        writer.write_all(b"dow\nignored").unwrap();
        assert_eq!(
            read_answer(&mut output, &mut buffer).unwrap(),
            Some("window".into())
        );
    }

    #[test]
    fn read_answer_accepts_empty_eof_as_dismissal() {
        let mut output = Cursor::new(Vec::new());
        assert_eq!(
            read_answer(&mut output, &mut Vec::new()).unwrap(),
            Some(String::new())
        );
    }

    #[test]
    fn read_answer_rejects_oversized_responses() {
        let mut output = Cursor::new(vec![b'x'; 129]);
        let error = read_answer(&mut output, &mut Vec::new()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn cancel_removes_the_source_and_releases_the_child() {
        let event_loop = calloop::EventLoop::<crate::test_support::TestWm>::try_new().unwrap();
        let mut host: MenuHost<()> = MenuHost::default();
        install(
            event_loop.handle(),
            &mut host,
            |wm: &mut crate::test_support::TestWm| wm.test_ctx(),
            |_, _, _, _| {},
        );
        let mut command = Command::new("sh");
        command
            .args(["-c", "exec sleep 30"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .process_group(0);
        start(&mut host, command, (), String::new()).unwrap();
        assert!(host.source.is_some());
        assert!(host.child.is_some());
        let generation = host.generation;
        host.cancel();
        assert!(host.source.is_none());
        assert!(host.child.is_none());
        assert_ne!(host.generation, generation);
    }

    #[test]
    fn finish_child_accepts_an_already_exited_process() {
        let mut child = Command::new("sh")
            .args(["-c", "exit 0"])
            .stdout(Stdio::piped())
            .process_group(0)
            .spawn()
            .unwrap();
        let _ = child.wait();
        // Reaping a completed child must not signal a reused process group.
        finish_child(child);
    }
}
