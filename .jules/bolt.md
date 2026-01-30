## 2026-01-30 - Heap Allocation in Hot Path
**Learning:** `drawstatusbar` in `bar.c` performs a `malloc` and `free` on every call to render the status text. This function is called frequently (e.g., on every status update or window focus change). Since the status text buffer is globally fixed at 1024 bytes, this allocation is unnecessary and adds overhead/fragmentation.
**Action:** Replace `malloc` with a stack-allocated buffer (small string optimization) for bounded strings in frequent rendering paths.
