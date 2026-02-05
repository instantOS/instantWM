## 2026-02-05 - Masked Global Variables in C
**Learning:** The global variable `stext` is masked by a function argument of the same name in `drawstatusbar(..., char *stext)`. This requires careful inspection of variable scope when optimizing, as `sizeof(stext)` yields different results (pointer size vs array size) depending on the scope.
**Action:** Always verify variable declarations and scope before applying size-based optimizations or `sizeof` checks in C codebases.
