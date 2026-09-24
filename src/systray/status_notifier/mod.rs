use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use calloop::ping::Ping;
use zbus::blocking::{Connection, MessageIterator, Proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedValue, Value};

use crate::systray::{
    MenuAction, MenuEntry, MenuToggle, MenuView, StatusNotifierItem, StatusNotifierTray,
};
use crate::types::{MonitorId, MouseButton, Point, Size, TagMask, WindowId};

const WATCHER_SERVICE: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

const ITEM_IFACE: &str = "org.kde.StatusNotifierItem";
const DBUSMENU_IFACE: &str = "com.canonical.dbusmenu";
const WORKER_RETRY_MIN: Duration = Duration::from_secs(1);
const WORKER_RETRY_MAX: Duration = Duration::from_secs(60);

/// How often to fall back to a full reconcile scan of StatusNotifierItems.
///
/// Item lifetime and icon changes are delivered as D-Bus signals (see
/// [`WatcherEvent`]), so this scan is only a safety net: it retries items
/// whose icon was not available at registration and covers items that do not
/// emit `NewIcon`. It runs at a slow, low-cost cadence.
const ICON_REFRESH_FALLBACK: Duration = Duration::from_secs(10);

mod discovery;
mod icon;
mod menu;
mod runtime;
mod watcher;
mod worker;
use discovery::*;
use icon::*;
use menu::*;
pub(crate) use runtime::StatusNotifierRuntime;
use watcher::*;
use worker::*;

/// Build a short-lived proxy without zbus' lazy property cache.
///
/// The worker reads individual properties while reconciling items and opening
/// menus. A property cache would fetch every property and install a signal
/// match for each short-lived proxy, delaying some implementations.
fn uncached_proxy<'a>(
    conn: &Connection,
    destination: &'a str,
    path: &'a str,
    interface: &'a str,
) -> zbus::Result<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(conn)
        .destination(destination)?
        .path(path)?
        .interface(interface)?
        .cache_properties(CacheProperties::No)
        .build()
}

/// A request expected to produce a native Wayland toplevel because the item
/// does not expose a host-renderable DBusMenu.
#[derive(Clone, Debug)]
pub(crate) struct NativeMenuRequest {
    pub created: Instant,
    pub anchor: Point,
    pub service: String,
    pub path: String,
    /// PID owning the D-Bus name, used to avoid claiming an unrelated
    /// toplevel that happens to map during the request timeout.
    pub owner_pid: Option<u32>,
}

impl NativeMenuRequest {
    pub(crate) fn matches_client_pid(&self, client_pid: Option<u32>) -> bool {
        self.owner_pid
            .is_some_and(|expected| client_pid == Some(expected))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveNativeMenu {
    pub win: WindowId,
    pub service: String,
    pub path: String,
    pub monitor_id: MonitorId,
    pub opened_tags: TagMask,
    pub close_requested: bool,
}

/// Cross-thread handoff for a pending native menu request.
pub(crate) type NativeMenuRequestSlot = Arc<Mutex<Option<NativeMenuRequest>>>;

#[cfg(test)]
mod tests;
