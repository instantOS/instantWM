## 2026-01-08 - Stack Allocation in Hot Path
**Learning:** In C window managers, status bars are redrawn very frequently (every second or more). Allocating strings on the heap (`malloc`) for temporary buffers in these hot paths adds unnecessary overhead and fragmentation.
**Action:** For bounded strings (like status text which is often limited to 1024 bytes), use stack allocation. It's faster, safer (no memory leaks), and removes failure paths (`die("malloc")`). Always check bounds with `strncpy`.
