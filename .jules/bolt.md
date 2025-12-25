## 2024-05-23 - X11 Sync vs Flush
**Learning:** In X11 window managers, `XSync(dpy, False)` is a blocking call that waits for a round-trip to the X server. Using `XFlush(dpy)` instead in high-frequency paths like `resizeclient` and `drw_map` significantly improves responsiveness by not waiting for confirmation.
**Action:** When optimizing X11 C code, look for unnecessary `XSync` calls in hot paths and replace them with `XFlush` unless the strict synchronization is required for correctness.
