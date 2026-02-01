## 2026-02-01 - Stack Allocation for Status Bar
**Learning:** `drawstatusbar` in `bar.c` is a hot path called frequently. It was using `malloc` for a string buffer (`stext`) that is globally bounded to 1024 bytes.
**Action:** Use stack allocation (`char buf[1024]`) for strings <= 1024 bytes to avoid malloc overhead and fragmentation. Add a fallback to `malloc` for safety.
