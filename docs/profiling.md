# Agent-friendly profiling

The primary instantWM profiling workflow uses Linux `perf`. It records the DRM
compositor, then produces source-linked text, JSON, and the raw capture. An
agent can inspect the first two formats without understanding a flame graph or
driving a profiler UI.

## Why perf

`perf` is the best default here because it can sample the compositor with low
overhead, unwind Rust stacks, retain the raw data, report exact source lines,
and emit a stable non-interactive report. Samply is a good optional viewer and
can import the same `perf.data`. `flamegraph` is useful for a human overview,
but its SVG is not the primary agent interface. Sysprof is valuable when a
whole-desktop or kernel-wide capture matters, but its native capture is less
convenient for automated text analysis.

This workflow measures CPU consumption. It does not directly measure GPU
execution time or explain wall-clock stalls while the compositor sleeps.
Vendor GPU tools, DRM tracepoints, or explicit tracing should be a separate
second-stage investigation after a CPU hotspot is identified.

## Prerequisites and permissions

Install `perf` from the package that matches the running kernel. `python3` and
`just` are also required. Samply is optional.

The capture profiles a child process and requests user-space samples only, so
Linux's standard `kernel.perf_event_paranoid=2` is sufficient. Some
distributions set a stricter value. Check and temporarily relax it with:

```sh
cat /proc/sys/kernel/perf_event_paranoid
just profile-permissions
```

The recipe changes the runtime sysctl to `2`; it does not create a persistent
system configuration and it does not run instantWM or `perf` as root.

## DRM and TTY requirement

Run DRM captures from an active local TTY, with no other compositor owning the
seat. Log in on the TTY, enter the repository, and run the recipe there. The
build itself works from any shell, but `instantwm --backend drm` normally needs
the active seat through libseat/logind. A remote shell, terminal inside another
compositor, or an agent command runner without the active seat may build and
analyze captures but should not be expected to start the DRM session.

This differs from the older end-to-end smoke test. `just e2e` starts the nested
Wayland/winit backend and must run inside an existing Wayland graphical session;
it does not exercise DRM/KMS. That test checks window lifecycle and tiling
geometry, while the profiler workload generates repeatable activity for a DRM
CPU capture. Both require that no other instantWM instance owns the default IPC
socket.

## Capture workflows

Build and run a 20-second scripted capture:

```sh
just profile
```

The `profiling` Cargo profile is optimized at level 2, keeps full debug info,
and is not a release build. The build also forces frame pointers for reliable,
low-overhead stack unwinding.

Choose another duration or do the interactions yourself:

```sh
just profile 45 standard
just profile 30 manual
```

The standard workload opens four clients and cycles through every layout while
updating the bar. Set `PROFILE_APP_CMD` if none of `foot`, `weston-terminal`,
`gtk4-demo`, `gtk3-demo`, or `xmessage` is suitable:

```sh
PROFILE_APP_CMD='my-wayland-test-client' just profile 30 standard
```

Manual input can be mixed into the standard workload. For pointer-heavy
testing, manual interaction is currently preferable: generic Wayland clients
cannot inject input into a compositor by design. A future dedicated test
protocol or a carefully isolated `uinput` harness would make pointer scripts
deterministic without relying on screen coordinates.

## Output contract

Each run creates `target/profiles/YYYYMMDD-HHMMSS/` and updates
`target/profiles/latest`. Start with:

- `summary.md`: the top source-linked self-CPU hotspots.
- `hotspots.json`: structured data with an explicit metric and schema version.
- `callgraph.txt`: caller/callee context in `perf report --stdio` form.
- `hotspots.tsv`: the complete flat perf table.
- `metadata.json`: backend, PID, duration, frequency, and workload.
- `instantwm.log` and `workload.log`: runtime evidence.
- `perf.data`: raw samples for deeper analysis.

Regenerate reports after changing the report script:

```sh
just profile-report target/profiles/latest
```

Source files and the profiling binary must remain available at their recorded
paths for reliable symbolization. Preserve `perf.data`, the corresponding
`target/profiling/instantwm`, and the Git revision when sharing a capture.

For optional browser-based exploration, convert the same recording without
starting a UI:

```sh
just profile-samply target/profiles/latest
```

This writes `samply-profile.json.gz`. A human can later use `samply load`.
FlameGraph can likewise consume the raw file with
`flamegraph --perfdata target/profiles/latest/perf.data`, but SVG is deliberately
not part of the agent-facing output contract.
