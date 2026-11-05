#!/usr/bin/env python3
"""Exercise XWayland dialogs, hung-client close and crash cleanup in a nested WM.

Requires a Wayland session, XWayland, libX11, and the debug instantwm binaries.
Run with python3 tests/xwayland_dialogs.py. No application crash is required.
Use --check-idle-cursor-updates to additionally exercise the pending upstream
Smithay flush fix; the currently pinned upstream revision can fail that check.
"""

import argparse
import ctypes as C
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import time


def wait_for(probe, description):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        result = probe()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError(f"timed out: {description}")


def client(directory):
    x = C.CDLL("libX11.so.6")
    display = C.c_void_p
    window = C.c_ulong
    signatures = {
        "XOpenDisplay": (display, [C.c_char_p]),
        "XDefaultRootWindow": (window, [display]),
        "XCreateSimpleWindow": (window, [display, window, C.c_int, C.c_int,
                                         C.c_uint, C.c_uint, C.c_uint, window, window]),
        "XInternAtom": (window, [display, C.c_char_p, C.c_int]),
        "XChangeProperty": (C.c_int, [display, window, window, window, C.c_int,
                                     C.c_int, C.c_void_p, C.c_int]),
        "XStoreName": (C.c_int, [display, window, C.c_char_p]),
        "XMapWindow": (C.c_int, [display, window]),
        "XSetTransientForHint": (C.c_int, [display, window, window]),
        "XSetWMProtocols": (C.c_int, [display, window, C.POINTER(window), C.c_int]),
        "XSync": (C.c_int, [display, C.c_int]),
        "XGetInputFocus": (C.c_int, [display, C.POINTER(window), C.POINTER(C.c_int)]),
        "XGetSelectionOwner": (window, [display, window]),
        "XGetWindowProperty": (C.c_int, [display, window, window, C.c_long,
            C.c_long, C.c_int, window, C.POINTER(window), C.POINTER(C.c_int),
            C.POINTER(C.c_ulong), C.POINTER(C.c_ulong), C.POINTER(C.c_void_p)]),
        "XFree": (C.c_int, [C.c_void_p]),
    }
    for name, (restype, argtypes) in signatures.items():
        getattr(x, name).restype = restype
        getattr(x, name).argtypes = argtypes
    class CursorImage(C.Structure):
        _fields_ = [("x", C.c_short), ("y", C.c_short),
                    ("width", C.c_ushort), ("height", C.c_ushort),
                    ("xhot", C.c_ushort), ("yhot", C.c_ushort),
                    ("serial", C.c_ulong), ("pixels", C.POINTER(C.c_ulong))]

    fixes = C.CDLL("libXfixes.so.3")
    fixes.XFixesGetCursorImage.argtypes = [display]
    fixes.XFixesGetCursorImage.restype = C.POINTER(CursorImage)
    d = x.XOpenDisplay(None)
    assert d, "cannot connect to nested XWayland"
    root = x.XDefaultRootWindow(d)

    def atom(name):
        return x.XInternAtom(d, name.encode(), False)

    def atoms(w, name, values):
        data = (window * len(values))(*(atom(value) for value in values))
        x.XChangeProperty(d, w, atom(name), atom("ATOM"), 32, 0, data, len(values))

    main = x.XCreateSimpleWindow(d, root, 50, 50, 600, 400, 0, 0, 0xffffff)
    dialog = x.XCreateSimpleWindow(d, root, 150, 150, 240, 160, 0, 0, 0xffffff)
    x.XStoreName(d, main, b"iwm-modal-main")
    x.XStoreName(d, dialog, b"iwm-modal-dialog")
    atoms(dialog, "_NET_WM_WINDOW_TYPE", ["_NET_WM_WINDOW_TYPE_DIALOG"])
    atoms(dialog, "_NET_WM_STATE", ["_NET_WM_STATE_ABOVE", "_NET_WM_STATE_MODAL"])
    x.XSetTransientForHint(d, dialog, main)
    # Advertise graceful close, but deliberately never handle it. Stopping this
    # process below then models an application hung in its event/render loop.
    protocols = (window * 1)(atom("WM_DELETE_WINDOW"))
    for w in (main, dialog):
        x.XSetWMProtocols(d, w, protocols, len(protocols))
    x.XMapWindow(d, main)
    x.XMapWindow(d, dialog)

    while True:
        x.XSync(d, False)
        focus, revert = window(), C.c_int()
        x.XGetInputFocus(d, C.byref(focus), C.byref(revert))
        owner = x.XGetSelectionOwner(d, atom("_XSETTINGS_S0"))
        settings = {}
        if owner:
            typ, fmt, count, remaining, data = window(), C.c_int(), window(), window(), C.c_void_p()
            x.XGetWindowProperty(d, owner, atom("_XSETTINGS_SETTINGS"), 0, 16384,
                False, 0, C.byref(typ), C.byref(fmt), C.byref(count),
                C.byref(remaining), C.byref(data))
            if data:
                raw = C.string_at(data, count.value)
                x.XFree(data)
                endian = "<" if raw[0] == 0 else ">"
                offset = 12
                for _ in range(struct.unpack_from(endian + "I", raw, 8)[0]):
                    kind, _, length = struct.unpack_from(endian + "BBH", raw, offset)
                    offset += 4
                    name = raw[offset:offset + length].decode()
                    offset = (offset + length + 3) & ~3
                    offset += 4  # last-change serial
                    if kind == 0:
                        value = struct.unpack_from(endian + "i", raw, offset)[0]
                        offset += 4
                    elif kind == 1:
                        length = struct.unpack_from(endian + "I", raw, offset)[0]
                        offset += 4
                        value = raw[offset:offset + length].decode()
                        offset = (offset + length + 3) & ~3
                    else:
                        offset += 8
                        continue
                    settings[name] = value
        image = fixes.XFixesGetCursorImage(d)
        cursor_size = [image.contents.width, image.contents.height]
        x.XFree(image)
        snapshot = {"cursor_size": cursor_size, "pid": os.getpid(), "display": os.environ["DISPLAY"], "focus": focus.value,
                    "dialog": dialog, "settings": settings}
        temporary = directory / "snapshot.tmp"
        temporary.write_text(json.dumps(snapshot))
        temporary.replace(directory / "snapshot.json")
        time.sleep(0.025)


def run(check_idle_cursor_updates=False, host_x11=False):
    assert os.environ.get("WAYLAND_DISPLAY"), "run inside a Wayland session"
    root = Path(__file__).resolve().parents[1]
    with tempfile.TemporaryDirectory(prefix="instantwm-dialogs-") as tmp:
        directory = Path(tmp)
        env = dict(os.environ, INSTANTWM_SOCKET_BIND=str(directory / "wm.sock"),
                   INSTANTWM_SOCKET=str(directory / "wm.sock"), INSTANTWM_TEST="1",
                   INSTANTWM_AUTOSTART="0", XCURSOR_THEME="Adwaita", XCURSOR_SIZE="24",
                   XDG_CONFIG_HOME=str(directory / "config"))
        config_dir = directory / "config" / "instantwm"
        config_dir.mkdir(parents=True)
        (config_dir / "config.toml").write_text("")
        log = (directory / "wm.log").open("w+")
        wm_env = dict(env)
        if host_x11:
            wm_env.pop("WAYLAND_DISPLAY", None)
        wm = subprocess.Popen([root / "target/debug/instantwm", "--backend", "nested"],
                              env=wm_env, stdout=log, stderr=log, start_new_session=True)

        def ctl(*args):
            assert wm.poll() is None, f"nested compositor exited with {wm.returncode}"
            result = subprocess.run([root / "target/debug/instantwmctl", "--json", *map(str, args)],
                                    env=env, capture_output=True, text=True, timeout=5)
            if result.returncode:
                return None
            return json.loads(result.stdout) if result.stdout.strip() else True

        def snapshot():
            path = directory / "snapshot.json"
            return json.loads(path.read_text()) if path.exists() else {}

        try:
            wait_for(lambda: ctl("status"), "nested compositor startup")
            assert ctl("config", "set", "window.focus_follows_mouse", "force") is not None
            assert ctl("config", "set", "window.focus_follows_float_mouse", "true") is not None
            assert ctl("spawn", "--", sys.executable, str(Path(__file__).resolve()),
                       "--client", tmp) is not None
            windows = wait_for(lambda: (rows if len(rows := ctl("window", "list") or []) == 2 else None),
                               "both X11 windows managed")
            main = next(w for w in windows if w["title"] == "iwm-modal-main")
            dialog = next(w for w in windows if w["title"] == "iwm-modal-dialog")
            wait_for(lambda: snapshot().get("settings", {}).get("Gtk/CursorThemeSize") == 24,
                     "initial XWayland cursor settings")
            time.sleep(0.5)  # settle spawn animations before using final IPC geometry
            assert ctl("test", "focus-window", main["id"]) is not None
            ctl("test", "pointer-move", 5, 5)
            dialog = next(w for w in ctl("window", "list") if w["id"] == dialog["id"])
            rect = dialog["geometry"]
            ctl("test", "pointer-move", rect["x"] + rect["w"] // 2,
                rect["y"] + rect["h"] // 2)
            wait_for(lambda: any(w["id"] == dialog["id"] and w["is_focused"]
                                 for w in ctl("window", "list") or []), "dialog hover focus")
            wait_for(lambda: snapshot().get("focus") == snapshot().get("dialog"),
                     "actual X11 input focus follows compositor focus")
            print("XWayland modal/above focus, X11 input focus and initial cursor settings: PASS")
            if check_idle_cursor_updates:
                time.sleep(1)  # drain focus events that could flush unrelated updates
                initial_cursor = snapshot()["cursor_size"]
                assert ctl("config", "set", "cursor.size", "32") is not None
                wait_for(lambda: snapshot().get("settings", {}).get("Gtk/CursorThemeSize") == 32,
                         "cursor config reload reaches XWayland")
                wait_for(lambda: snapshot().get("cursor_size") != initial_cursor,
                         "default X11 cursor image changes while idle")
                print("Idle XWayland cursor image and settings reload: PASS")

            # Close requests must not wait for the app, including when repeated
            # while a modal dialog is focused. IPC uses the same close funnel
            # as Super+Q, but targets only this isolated test application.
            pid = snapshot()["pid"]
            os.kill(pid, signal.SIGSTOP)
            slowest = 0
            for _ in range(30):
                started = time.monotonic()
                assert ctl("window", "close", dialog["id"]) is not None
                assert ctl("test", "pointer-move", 5, 5) is not None
                assert ctl("status") is not None
                slowest = max(slowest, time.monotonic() - started)
            assert len(ctl("window", "list")) == 2, "hung client must remain managed"
            print(f"Hung XWayland client, repeated close and pointer commands: PASS "
                  f"(slowest round trip {slowest:.3f}s)")

            # A crash destroys both the parent and modal dialog together.
            os.kill(pid, signal.SIGKILL)
            wait_for(lambda: ctl("window", "list") == [], "crashed client's windows removed")
            assert ctl("test", "pointer-move", 10, 10) is not None
            assert ctl("status") is not None

            # XWayland and the compositor must still accept and focus clients
            # after cleanup, rather than only retaining an empty IPC model.
            (directory / "snapshot.json").unlink(missing_ok=True)
            assert ctl("spawn", "--", sys.executable, str(Path(__file__).resolve()),
                       "--client", tmp) is not None
            wait_for(lambda: len(ctl("window", "list") or []) == 2,
                     "new windows managed after crash")
            wait_for(lambda: snapshot().get("focus") == snapshot().get("dialog")
                     and snapshot().get("dialog"), "new dialog receives X11 input focus")
            print("XWayland crash cleanup and subsequent client focus: PASS")
        except BaseException:
            log.seek(0)
            print(log.read(), file=sys.stderr)
            raise
        finally:
            if pid := snapshot().get("pid"):
                try:
                    os.kill(pid, signal.SIGCONT)
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            os.killpg(wm.pid, signal.SIGTERM)
            try:
                wm.wait(timeout=5)
            except subprocess.TimeoutExpired:
                os.killpg(wm.pid, signal.SIGKILL)
                wm.wait()
            log.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--client", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--check-idle-cursor-updates", action="store_true",
                        help="also check the pending upstream Smithay flush fix")
    parser.add_argument("--host-x11", action="store_true",
                        help="host the nested compositor through the session's XWayland")
    args = parser.parse_args()
    if args.client:
        client(args.client)
    else:
        run(args.check_idle_cursor_updates, args.host_x11)
