## 2024-10-23 - Blocking Animations
**Learning:** The `animateclient` function uses `usleep(15000)` inside a `while` loop, which blocks the single-threaded X11 event loop. This causes the entire window manager to freeze during any window animation.
**Action:** Future optimizations should focus on refactoring this to use an event-based non-blocking animation system, possibly using a timer or `XNextEvent` with timeouts.
