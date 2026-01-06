# Bolt's Journal

## 2024-10-26 - XSync vs XFlush
**Learning:** `XSync` forces a round-trip to the X server, blocking until all requests are processed. `XFlush` just flushes the output buffer. Using `XSync` in frequent operations (like drawing) kills performance.
**Action:** Prefer `XFlush` for drawing operations, only use `XSync` when error handling or strict ordering is absolutely required.
