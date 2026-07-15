#!/usr/bin/env python3
"""Turn perf.data into compact, stable files that an agent can inspect."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def fail(message: str) -> None:
    print(f"profile-report: {message}", file=sys.stderr)
    raise SystemExit(1)


capture = Path(sys.argv[1] if len(sys.argv) > 1 else "target/profiles/latest").resolve()
perf_data = capture if capture.is_file() else capture / "perf.data"
capture_dir = perf_data.parent
if not perf_data.is_file():
    fail(f"missing {perf_data}")

metadata_path = capture_dir / "metadata.json"
metadata = json.loads(metadata_path.read_text()) if metadata_path.is_file() else {}
gpu_summary_path = capture_dir / "gpu-summary.json"
gpu_summary = (
    json.loads(gpu_summary_path.read_text()) if gpu_summary_path.is_file() else None
)
pid = metadata.get("pid")
pid_args = ["--pid", str(pid)] if pid else []

base = [
    "perf",
    "report",
    "--input",
    str(perf_data),
    "--stdio",
    "--stdio-color",
    "never",
]
flat_cmd = (
    base
    + pid_args
    + [
        "--call-graph",
        "none",
        "--no-inline",
        "--no-children",
        "--percent-limit",
        "0.05",
        "--full-source-path",
        "--sort",
        "comm,dso,symbol,srcline",
        "--fields",
        "overhead,sample,comm,dso,symbol,srcline",
        "--field-separator",
        "\t",
    ]
)
print(
    "profile-report: resolving source-linked hotspots "
    "(large captures can take a while)",
    flush=True,
)
flat = subprocess.run(flat_cmd, check=True, text=True, capture_output=True).stdout
(capture_dir / "hotspots.tsv").write_text(flat)

hotspots: list[dict[str, object]] = []
for line in flat.splitlines():
    if not line or line.startswith("#") or "\t" not in line:
        continue
    fields = [field.strip() for field in line.split("\t")]
    if len(fields) < 6 or not fields[0].endswith("%"):
        continue
    try:
        overhead = float(fields[0][:-1])
        samples = int(fields[1])
    except ValueError:
        continue
    hotspots.append(
        {
            "self_cpu_percent": overhead,
            "samples": samples,
            "thread": fields[2],
            "object": fields[3],
            "symbol": fields[4].removeprefix("[.] ").removeprefix("[k] "),
            "source": fields[5],
        }
    )

hotspots.sort(
    key=lambda item: (-float(item["self_cpu_percent"]), -int(item["samples"]))
)

payload = {
    "schema_version": 1,
    "metric": "sampled user-space CPU self time",
    "capture": metadata,
    "hotspots": hotspots,
}
(capture_dir / "hotspots.json").write_text(json.dumps(payload, indent=2) + "\n")

callgraph_cmd = (
    base
    + pid_args
    + [
        "--percent-limit",
        "0.5",
        "--no-inline",
        "--sort",
        "comm,dso,symbol",
        "--call-graph",
        "graph,0.5,caller,function,percent",
    ]
)
print("profile-report: building textual call graph", flush=True)
callgraph = subprocess.run(
    callgraph_cmd, check=True, text=True, capture_output=True
).stdout
(capture_dir / "callgraph.txt").write_text(callgraph)


def cell(value: object) -> str:
    return str(value).replace("|", "\\|").replace("\n", " ")


lines = [
    "# instantWM CPU profile",
    "",
    f"Capture: `{capture_dir}`",
    "",
    "Metric: sampled user-space CPU self time. Percentages are CPU samples, not wall-clock latency.",
    "",
    "| Self CPU | Samples | Symbol | Source | Object |",
    "|---:|---:|---|---|---|",
]
for item in hotspots[:40]:
    lines.append(
        f"| {item['self_cpu_percent']:.2f}% | {item['samples']} | "
        f"{cell(item['symbol'])} | {cell(item['source'])} | {cell(item['object'])} |"
    )
if gpu_summary is not None:
    lines += ["", "## GPU usage", ""]
    if not gpu_summary["available"]:
        lines.append(
            "DRM fdinfo did not expose client GPU statistics for this capture."
        )
    elif not gpu_summary["engines"]:
        lines.append(
            "DRM client statistics were available, but the driver exposed no engine-busy counters."
        )
    else:
        lines += [
            "| Engine | Average busy | Peak interval | Busy time |",
            "|---|---:|---:|---:|",
        ]
        for engine, values in gpu_summary["engines"].items():
            lines.append(
                f"| {cell(engine)} | {values['average_busy_percent']:.2f}% | "
                f"{values['peak_interval_percent']:.2f}% | {values['busy_seconds']:.4f}s |"
            )
lines += [
    "",
    "Use `hotspots.json` for structured data and `callgraph.txt` to distinguish expensive callees from their callers.",
    "The raw `perf.data` remains available for `perf annotate`, Samply, or FlameGraph.",
    "",
]
(capture_dir / "summary.md").write_text("\n".join(lines))
print(f"profile-report: wrote {capture_dir / 'summary.md'}")
