#!/usr/bin/env python3
"""Sample standardized DRM fdinfo counters for one process."""

from __future__ import annotations

import json
import signal
import sys
import time
from pathlib import Path


if len(sys.argv) != 4:
    print("usage: profile-gpu.py PID CAPTURE_DIR INTERVAL_MS", file=sys.stderr)
    raise SystemExit(2)

pid = int(sys.argv[1])
capture_dir = Path(sys.argv[2])
interval = int(sys.argv[3]) / 1000
if interval <= 0:
    raise SystemExit("INTERVAL_MS must be positive")

running = True


def stop(_signum: int, _frame: object) -> None:
    global running
    running = False


signal.signal(signal.SIGINT, stop)
signal.signal(signal.SIGTERM, stop)


def scaled_bytes(value: str) -> int | None:
    parts = value.split()
    try:
        number = int(parts[0])
    except (IndexError, ValueError):
        return None
    scale = {"B": 1, "KiB": 1024, "MiB": 1024**2, "GiB": 1024**3}
    return number * scale.get(parts[1] if len(parts) > 1 else "B", 1)


def read_clients() -> list[dict[str, object]]:
    clients: dict[tuple[str, str, str], dict[str, object]] = {}
    for fdinfo in Path(f"/proc/{pid}/fdinfo").glob("*"):
        try:
            rows = dict(
                line.split(":", 1)
                for line in fdinfo.read_text().splitlines()
                if ":" in line
            )
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue
        rows = {key.strip(): value.strip() for key, value in rows.items()}
        driver = rows.get("drm-driver")
        client_id = rows.get("drm-client-id")
        if not driver or not client_id:
            continue
        device = rows.get("drm-pdev", "unknown")
        key = (driver, device, client_id)
        if key in clients:  # Duplicated file descriptors represent one DRM client.
            continue
        engines: dict[str, int] = {}
        capacities: dict[str, int] = {}
        memory: dict[str, int] = {}
        for name, value in rows.items():
            if name.startswith("drm-engine-capacity-"):
                try:
                    capacities[name.removeprefix("drm-engine-capacity-")] = int(value)
                except ValueError:
                    pass
            elif name.startswith("drm-engine-"):
                try:
                    engines[name.removeprefix("drm-engine-")] = int(value.split()[0])
                except (IndexError, ValueError):
                    pass
            elif name.startswith("drm-memory-"):
                parsed = scaled_bytes(value)
                if parsed is not None:
                    memory[name.removeprefix("drm-memory-")] = parsed
        clients[key] = {
            "driver": driver,
            "device": device,
            "client_id": client_id,
            "engines_ns": engines,
            "engine_capacity": capacities,
            "memory_bytes": memory,
        }
    return list(clients.values())


started_ns = time.monotonic_ns()
samples: list[dict[str, object]] = []
samples_path = capture_dir / "gpu-samples.jsonl"
with samples_path.open("w") as output:
    while running and Path(f"/proc/{pid}").exists():
        now_ns = time.monotonic_ns()
        sample = {
            "elapsed_seconds": round((now_ns - started_ns) / 1e9, 6),
            "clients": read_clients(),
        }
        samples.append(sample)
        output.write(json.dumps(sample, separators=(",", ":")) + "\n")
        output.flush()
        time.sleep(interval)


def totals(sample: dict[str, object], field: str) -> dict[str, int]:
    result: dict[str, int] = {}
    for client in sample["clients"]:  # type: ignore[union-attr]
        prefix = f"{client['driver']}:{client['device']}:"  # type: ignore[index]
        for name, value in client[field].items():  # type: ignore[index,union-attr]
            result[prefix + name] = result.get(prefix + name, 0) + int(value)
    return result


engine_busy_ns: dict[str, int] = {}
engine_peak_percent: dict[str, float] = {}
memory_peak_bytes: dict[str, int] = {}
for sample in samples:
    for name, value in totals(sample, "memory_bytes").items():
        memory_peak_bytes[name] = max(memory_peak_bytes.get(name, 0), value)

for previous, current in zip(samples, samples[1:], strict=False):
    elapsed_ns = int(
        (float(current["elapsed_seconds"]) - float(previous["elapsed_seconds"])) * 1e9
    )
    if elapsed_ns <= 0:
        continue
    before = totals(previous, "engines_ns")
    after = totals(current, "engines_ns")
    for name, value in after.items():
        delta = max(0, value - before.get(name, value))
        engine_busy_ns[name] = engine_busy_ns.get(name, 0) + delta
        utilization = 100 * delta / elapsed_ns
        engine_peak_percent[name] = max(engine_peak_percent.get(name, 0), utilization)

span_seconds = (
    float(samples[-1]["elapsed_seconds"]) - float(samples[0]["elapsed_seconds"])
    if len(samples) > 1
    else 0.0
)
summary = {
    "schema_version": 1,
    "source": "Linux DRM fdinfo for the instantWM process",
    "available": any(sample["clients"] for sample in samples),
    "sample_interval_ms": int(interval * 1000),
    "sample_count": len(samples),
    "span_seconds": round(span_seconds, 6),
    "engines": {
        name: {
            "busy_seconds": round(busy_ns / 1e9, 6),
            "average_busy_percent": round(100 * busy_ns / (span_seconds * 1e9), 3)
            if span_seconds > 0
            else 0,
            "peak_interval_percent": round(engine_peak_percent.get(name, 0), 3),
        }
        for name, busy_ns in sorted(engine_busy_ns.items())
    },
    "peak_memory_bytes": dict(sorted(memory_peak_bytes.items())),
}
(capture_dir / "gpu-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
