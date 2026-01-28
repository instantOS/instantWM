# Bolt's Journal ⚡

## 2024-05-23 - [Blocking XSync in Drawing Loop]
**Learning:** `XSync` forces a round-trip to the X server, blocking the client until the server processes the request. In a drawing loop like `drw_map`, this adds significant latency (up to network RTT) per frame/update.
**Action:** Use `XCopyArea` without explicit `XSync` for drawing operations. `XNextEvent` (or explicit `XFlush`) will handle buffer flushing asynchronously.
