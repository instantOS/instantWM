#!/usr/bin/env python3
"""X11 capture regressions on a private Xvfb (debug binaries must be built).

Requires Xvfb, xdotool, xmessage, xprop and an icon font for the normal bar.
No running desktop is needed. All child processes and input belong to our Xvfb.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def wait_for(check, description, timeout=8):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        result = check()
        if result:
            return result
        time.sleep(0.05)
    raise AssertionError(f"Timed out: {description}")


def main():
    for tool in ("Xvfb", "xdotool", "xmessage", "xprop"):
        if not shutil.which(tool):
            raise SystemExit(f"Missing test dependency: {tool}")
    processes = []
    with tempfile.TemporaryDirectory(prefix="iwm-x11-", dir=os.environ.get("TMPDIR")) as directory:
        directory = Path(directory)
        env = dict(os.environ, INSTANTWM_AUTOSTART="0", INSTANTWM_LOG="warn",
                   INSTANTWM_SOCKET_BIND=str(directory / "wm.sock"),
                   INSTANTWM_SOCKET=str(directory / "wm.sock"),
                   XDG_CONFIG_HOME=str(directory / "config"))
        with (directory / "display").open("w+") as display, (directory / "wm.log").open("w+") as log:
            try:
                server = subprocess.Popen(
                    ["Xvfb", "-displayfd", str(display.fileno()), "-screen", "0", "1280x800x24", "-nolisten", "tcp"],
                    pass_fds=(display.fileno(),), stdout=log, stderr=log)
                processes.append(server)
                number = wait_for(lambda: (directory / "display").read_text().strip(), "Xvfb startup")
                env["DISPLAY"] = f":{number}"

                def run(*args, timeout=4, check=True):
                    return subprocess.run(args, env=env, text=True, capture_output=True,
                                          timeout=timeout, check=check)

                def ctl(*args, timeout=4):
                    return run(str(ROOT / "target/debug/instantwmctl"), *args, timeout=timeout)

                def windows():
                    return json.loads(ctl("--json", "window", "list").stdout)

                wm = subprocess.Popen([str(ROOT / "target/debug/instantwm"), "--backend", "x11"],
                                      env=env, stdout=log, stderr=log)
                processes.append(wm)

                def ready():
                    assert wm.poll() is None, "WM exited during startup"
                    try:
                        return ctl("status", timeout=0.3).returncode == 0
                    except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                        return False

                wait_for(ready, "WM startup", 20)

                def spawn(name, during_capture=False):
                    process = subprocess.Popen(["xmessage", "-name", name, "hello"], env=env,
                                               stdout=log, stderr=log)
                    processes.append(process)
                    def find():
                        result = run("xdotool", "search", "--name", f"^{name}$", check=False)
                        return int(result.stdout.splitlines()[0]) if result.stdout.strip() else None
                    window = wait_for(find, f"{name} X window")
                    if not during_capture:
                        wait_for(lambda: any(w["id"] == window for w in windows()), f"{name} managed")
                    return process, window

                def captured():
                    # A held modal grab keeps the WM's IPC unanswerable
                    # indefinitely, so require two consecutive timeouts: a
                    # one-off scheduling stall must not read as a capture.
                    for _ in range(2):
                        try:
                            ctl("status", timeout=0.25)
                        except subprocess.TimeoutExpired:
                            continue
                        return False
                    return True

                def begin(window):
                    ctl("window", "focus", str(window))
                    geometry = next(w["geometry"] for w in windows() if w["id"] == window)
                    x, y = geometry["x"] + geometry["width"] // 2, geometry["y"] + geometry["height"] // 2
                    run("xdotool", "mousemove", str(x), str(y), "keydown", "Super_L", "mousedown", "1")
                    time.sleep(0.1)
                    # Keep the pointer moving while polling for engagement: a
                    # busy WM can process the press after the first follow-up
                    # motion, so no single motion sample may be load-bearing.
                    deadline = time.monotonic() + 5
                    offset = 70
                    while not captured():
                        if time.monotonic() > deadline:
                            raise AssertionError("Test did not engage the modal pointer capture")
                        offset += 10
                        run("xdotool", "mousemove", str(x + offset), str(y + offset // 2))

                def release():
                    run("xdotool", "mouseup", "1", "keyup", "Super_L")
                    wait_for(ready, "main loop after release")

                def client_ids():
                    # Root EWMH state is observable even while IPC is blocked by capture.
                    result = run("xprop", "-root", "_NET_CLIENT_LIST").stdout
                    return [int(token.strip(","), 16) for token in result.split() if token.startswith("0x")]

                # Mapping and property changes must survive capture. Removing an
                # unrelated window must not end the original interaction.
                first, first_id = spawn("iwm-drag-primary")
                begin(first_id)
                second, second_id = spawn("iwm-drag-secondary", during_capture=True)
                wait_for(lambda: second_id in client_ids(), "MapRequest forwarded during capture")
                run("xprop", "-id", str(first_id), "-f", "_NET_WM_NAME", "8u", "-set", "_NET_WM_NAME", "updated-during-drag")
                second.terminate()
                second.wait(timeout=3)
                wait_for(lambda: second_id not in client_ids(), "unrelated destruction")
                assert captured(), "Unrelated destruction cancelled the dragged window"
                release()
                wait_for(lambda: any(w["title"] == "updated-during-drag" for w in windows()), "PropertyNotify forwarded")
                first.terminate()
                first.wait(timeout=3)
                wait_for(lambda: not windows(), "control removal")
                print("PASS: map/property forwarding and unrelated-window removal")

                # Both destroy and unmap must remove the target and return to the
                # main loop before mouse-up; subsequent motion/release must be safe.
                for operation in ("destroy", "unmap"):
                    process, window = spawn(f"iwm-drag-{operation}")
                    begin(window)
                    if operation == "destroy":
                        process.kill()
                        process.wait(timeout=3)
                    else:
                        run("xdotool", "windowunmap", str(window))
                    wait_for(ready, f"capture cancelled after {operation}, before mouse-up")
                    assert window not in [w["id"] for w in windows()], f"Ghost after {operation}"
                    run("xdotool", "mousemove", "800", "600")
                    release()
                    assert not windows(), f"Ghost after release following {operation}"
                    if process.poll() is None:
                        process.terminate()
                        process.wait(timeout=3)
                    print(f"PASS: {operation} cancels capture and removes client before mouse-up")

                # A keybind dispatched inside the grab loop that hides the
                # dragged window (Super+2 views another tag) must cancel the
                # capture instead of steering the parked window.
                process, window = spawn("iwm-drag-keybind")
                begin(window)

                def parked_off_screen():
                    result = run("xdotool", "getwindowgeometry", "--shell", str(window), check=False)
                    fields = dict(line.split("=", 1) for line in result.stdout.splitlines() if "=" in line)
                    return int(fields["X"]) < 0

                run("xdotool", "key", "2")  # Super is still held by begin()
                wait_for(ready, "capture cancelled after tag-switch keybind, before mouse-up")
                desktop = run("xprop", "-root", "_NET_CURRENT_DESKTOP").stdout
                assert desktop.strip().endswith("= 1"), f"Tag switch did not apply: {desktop}"
                wait_for(parked_off_screen, "target parked off-screen by tag switch")
                assert any(w["id"] == window for w in windows()), "Client lost after tag-switch keybind"
                run("xdotool", "mousemove", "800", "600")
                assert parked_off_screen(), "Drag motion still applied after the tag switch"
                release()
                # The window must have survived the cancelled drag, hidden but
                # managed: switch back to its tag and expect it back on screen.
                run("xdotool", "keydown", "Super_L")
                run("xdotool", "key", "1")
                run("xdotool", "keyup", "Super_L")
                wait_for(lambda: not parked_off_screen(),
                         "window back on screen after returning to its tag")
                process.terminate()
                process.wait(timeout=3)
                print("PASS: mid-drag tag-switch keybind cancels capture and keeps the client")
            except BaseException:
                log.flush()
                print((directory / "wm.log").read_text())
                raise
            finally:
                # Never use process-name matching: only terminate children we own.
                for process in reversed(processes):
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=3)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()


if __name__ == "__main__":
    main()
