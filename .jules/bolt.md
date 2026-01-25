## 2026-01-25 - Stack vs Heap in Hot Paths
**Learning:** `drawstatusbar` in `bar.c` allocates and frees a buffer on every redraw. Since the buffer size is bounded by `stext` (1024 bytes), this is unnecessary overhead.
**Action:** Use stack allocation for small, bounded buffers in frequent operations to avoid allocator overhead and fragmentation.
