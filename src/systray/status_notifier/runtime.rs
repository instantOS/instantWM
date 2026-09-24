use super::*;

pub(crate) struct StatusNotifierRuntime {
    pub(super) worker: Option<StatusNotifierWorker>,
    pub(super) restart_at: Option<Instant>,
    pub(super) retry_delay: Duration,
    pub(super) next_menu_session_id: AtomicU64,
    pub(super) native_menu_request: Option<NativeMenuRequestSlot>,
    pub(super) wake: Option<Ping>,
}

impl StatusNotifierRuntime {
    pub(crate) fn start(
        native_menu_request: Option<NativeMenuRequestSlot>,
        wake: Option<Ping>,
    ) -> Self {
        let mut runtime = Self {
            worker: None,
            restart_at: None,
            retry_delay: WORKER_RETRY_MIN,
            next_menu_session_id: AtomicU64::new(1),
            native_menu_request,
            wake,
        };
        match StatusNotifierWorker::spawn(runtime.native_menu_request.clone(), runtime.wake.clone())
        {
            Ok(worker) => runtime.worker = Some(worker),
            Err(error) => {
                log::warn!("status notifier: failed to spawn thread: {error}");
                runtime.schedule_restart();
            }
        }
        runtime
    }

    pub(crate) fn poll_events(
        &mut self,
        tray: &mut StatusNotifierTray,
        menu: &mut crate::systray::TrayMenuState,
    ) -> bool {
        let mut changed = false;
        let mut worker_stopped = false;
        if let Some(worker) = self.worker.as_ref() {
            loop {
                match worker.evt_rx.try_recv() {
                    Ok(SystrayEvt::Ready) => self.retry_delay = WORKER_RETRY_MIN,
                    Ok(SystrayEvt::ItemUpsert(item)) => {
                        changed |= upsert_item(tray, item);
                    }
                    Ok(SystrayEvt::ItemRemoved(service, path)) => {
                        let before = tray.items.len();
                        tray.items
                            .retain(|it| !(it.service == service && it.path == path));
                        changed |= tray.items.len() != before;
                    }
                    Ok(SystrayEvt::MenuChanged { session_id, view }) => {
                        changed |= menu.apply(session_id, view);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        worker_stopped = true;
                        break;
                    }
                }
            }
            worker_stopped |= worker.thread.is_finished();
        }

        if worker_stopped {
            self.handle_worker_exit();
            changed |= !tray.items.is_empty();
            tray.items.clear();
            changed |= menu.close().is_some();
        }

        if self.worker.is_none()
            && self
                .restart_at
                .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.restart_worker();
        }
        changed
    }

    fn handle_worker_exit(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let StatusNotifierWorker {
            cmd_tx,
            evt_rx,
            thread,
        } = worker;
        drop(cmd_tx);
        drop(evt_rx);
        match thread.join() {
            Ok(()) => log::warn!("status notifier: worker stopped; scheduling restart"),
            Err(payload) => log::error!(
                "status notifier: worker panicked: {}; scheduling restart",
                panic_message(payload.as_ref())
            ),
        }
        self.schedule_restart();
    }

    pub(super) fn schedule_restart(&mut self) {
        self.restart_at = Some(Instant::now() + self.retry_delay);
        self.retry_delay = (self.retry_delay * 2).min(WORKER_RETRY_MAX);
    }

    fn restart_worker(&mut self) {
        match StatusNotifierWorker::spawn(self.native_menu_request.clone(), self.wake.clone()) {
            Ok(worker) => {
                log::info!("status notifier: restarting worker");
                self.worker = Some(worker);
                self.restart_at = None;
            }
            Err(error) => {
                log::warn!("status notifier: failed to restart worker: {error}");
                self.schedule_restart();
            }
        }
    }

    pub fn dispatch_click_item(
        &self,
        service: String,
        path: String,
        button: MouseButton,
        position: Point,
    ) -> Option<u64> {
        let mut menu_session_id = None;
        let cmd = match button {
            MouseButton::Left => SystrayCmd::Activate {
                service,
                path,
                position,
            },
            MouseButton::Middle => SystrayCmd::SecondaryActivate {
                service,
                path,
                position,
            },
            MouseButton::Right => {
                let session_id = self.next_menu_session_id.fetch_add(1, Ordering::Relaxed);
                menu_session_id = Some(session_id);
                SystrayCmd::ContextMenu {
                    session_id,
                    service,
                    path,
                    position,
                }
            }
            _ => return None,
        };

        let sent = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.cmd_tx.send(cmd).is_ok());
        if !sent {
            return None;
        }
        menu_session_id
    }

    pub(crate) fn dispatch_menu_action(&self, session_id: u64, action: MenuAction) {
        if let Some(worker) = self.worker.as_ref() {
            let _ = worker
                .cmd_tx
                .send(SystrayCmd::MenuAction { session_id, action });
        }
    }

    pub(crate) fn close_menu(&self, session_id: u64) {
        if let Some(worker) = self.worker.as_ref() {
            let _ = worker.cmd_tx.send(SystrayCmd::CloseMenu { session_id });
        }
    }
}

pub(super) fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("non-string panic payload")
}
