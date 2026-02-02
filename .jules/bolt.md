## 2026-02-02 - Heap Allocation in Hot Path (drawstatusbar)
**Learning:** The `drawstatusbar` function allocates memory via `malloc` on every call to process the status text. Since the status text is typically small (capped at 1024 bytes by `stext`) and updated frequently (e.g., every second), this creates unnecessary heap churn.
**Action:** Use a stack buffer (Small String Optimization) for typical sizes and fallback to heap only when necessary. This reduces allocator overhead in the rendering loop.
