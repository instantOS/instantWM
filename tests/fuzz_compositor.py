#!/usr/bin/env python3
"""Watchdog, replay and reduction for instantwm-fuzz; run --help for commands.

Build: cargo build --features fuzzing --bin instantwm-fuzz
Smoke: python3 tests/fuzz_compositor.py campaign --cases 100 --output /tmp/iwm-fuzz
Replay: python3 tests/fuzz_compositor.py replay /tmp/iwm-fuzz/seed-0/case.json
Reduce: python3 tests/fuzz_compositor.py minimize path/to/case.json --output /tmp/reduced

This headless suite opens no display, graphics device, host IPC or D-Bus session.
GPU/DRM behavior is deliberately outside its claims. Each case gets a fresh
process; the watchdog remains outside the compositor's event loop.
"""

import argparse
import json
import math
import os
from pathlib import Path
import platform
import re
import resource
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = ROOT / "target/debug/instantwm-fuzz"


def positive(value):
    number = float(value)
    if not math.isfinite(number) or number <= 0:
        raise argparse.ArgumentTypeError("must be finite and positive")
    return number


def limits(memory_mib, timeout):
    def apply():
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        resource.setrlimit(resource.RLIMIT_NOFILE, (256, 256))
        memory = memory_mib * 1024 * 1024
        resource.setrlimit(resource.RLIMIT_AS, (memory, memory))
        resource.setrlimit(resource.RLIMIT_FSIZE, (16 * 1024 * 1024, 16 * 1024 * 1024))
        cpu = max(1, math.ceil(timeout))
        resource.setrlimit(resource.RLIMIT_CPU, (cpu, cpu + 1))
    return apply


def failure_signature(returncode, stderr):
    # Preserve panic location + invariant text so reduction cannot silently
    # replace the original bug with a different, easier crash.
    match = re.search(r"panicked at ([^\n]+)\n([^\n]+)", stderr)
    if match:
        return "panic:" + match.group(1) + ":" + match.group(2)
    return f"exit:{returncode}"


def execute(args, scenario, directory):
    directory.mkdir(parents=True, exist_ok=True)
    case = directory / "case.json"
    case.write_text(json.dumps(scenario, indent=2) + "\n")
    env = dict(os.environ)
    # The worker is headless and never publishes session state. Removing host
    # discovery variables also makes accidental future regressions fail closed.
    for name in ("DISPLAY", "WAYLAND_DISPLAY", "INSTANTWM_SOCKET", "INSTANTWM_SOCKET_BIND",
                 "DBUS_SESSION_BUS_ADDRESS", "DESKTOP_STARTUP_ID"):
        env.pop(name, None)
    runtime = directory / "runtime"
    runtime.mkdir(mode=0o700, exist_ok=True)
    env["XDG_RUNTIME_DIR"] = str(runtime.resolve())
    env["XDG_CONFIG_HOME"] = str(directory / "config")
    env["INSTANTWM_AUTOSTART"] = "0"
    started = time.monotonic()
    signature, report = None, None
    with (directory / "stdout.log").open("w") as out, (directory / "stderr.log").open("w") as err:
        worker = subprocess.Popen([str(args.binary), "--replay", str(case.resolve())],
            env=env, stdout=out, stderr=err, start_new_session=True,
            preexec_fn=limits(args.memory_mib, args.timeout))
        try:
            worker.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            signature = "timeout"
            try:
                os.killpg(worker.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            worker.wait()
    stderr = (directory / "stderr.log").read_text(errors="replace")
    if signature is None and worker.returncode:
        signature = failure_signature(worker.returncode, stderr)
    if signature is None:
        try:
            report = json.loads((directory / "stdout.log").read_text())
            assert report["actions"] == len(scenario["actions"])
            assert report["witness_roundtrips"] >= 2
            assert report["mapped_windows"] >= 1
            if report["max_roundtrip_micros"] > args.max_latency_ms * 1000:
                signature = "witness-latency"
        except (ValueError, KeyError, AssertionError, TypeError):
            signature = "invalid-worker-report"
    result = dict(signature=signature, returncode=worker.returncode,
                  elapsed_seconds=time.monotonic() - started, report=report)
    (directory / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def read_case(path):
    if path.stat().st_size > 1024 * 1024:
        raise ValueError("case exceeds 1 MiB")
    case = json.loads(path.read_text())
    if not isinstance(case, dict) or set(case) != {"actions"} or not isinstance(case["actions"], list):
        raise ValueError("expected an object containing an actions array")
    if len(case["actions"]) > 512:
        raise ValueError("case exceeds 512 actions")
    return case


def generated_case(args, seed):
    # Generation uses the worker's stable decoder, rather than duplicating a
    # PRNG/action mapping in Python. Save the exact result before execution.
    with tempfile.TemporaryDirectory(prefix="instantwm-fuzz-generate-") as tmp:
        path = Path(tmp) / "case.json"
        subprocess.run([str(args.binary), "--seed", str(seed), "--steps", str(args.steps),
                        "--emit", str(path)], check=True, timeout=args.timeout,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        return read_case(path)


def minimize(args, original):
    baseline = execute(args, original, args.output / "original")
    signature = baseline["signature"]
    if signature is None:
        raise ValueError("case passes; refusing to minimize a non-failure")
    attempts = 0

    def reproduces(actions):
        nonlocal attempts
        for _ in range(args.confirm):
            if attempts >= args.max_attempts:
                return False
            attempts += 1
            result = execute(args, {"actions": actions}, args.output / "attempt")
            if result["signature"] != signature:
                return False
        return True

    if not reproduces(original["actions"]):
        raise ValueError("failure is not repeatable with the same signature")
    actions = original["actions"]
    partitions = 2
    while actions and attempts < args.max_attempts:
        width = max(1, math.ceil(len(actions) / partitions))
        reduced = False
        for start in range(0, len(actions), width):
            candidate = actions[:start] + actions[start + width:]
            if reproduces(candidate):
                actions = candidate
                partitions = max(2, partitions - 1)
                reduced = True
                print(f"reduced to {len(actions)} actions ({attempts} executions)", flush=True)
                break
        if not reduced:
            if partitions >= len(actions):
                break
            partitions = min(len(actions), partitions * 2)
    minimized = {"actions": actions}
    path = args.output / "minimized.json"
    path.write_text(json.dumps(minimized, indent=2) + "\n")
    final = execute(args, minimized, args.output / "verification")
    if final["signature"] != signature:
        raise ValueError("final reproduction changed; original case is preserved")
    print(f"Saved {path} ({signature}; {attempts} reduction executions)")
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("mode", choices=["campaign", "replay", "minimize"])
    parser.add_argument("case", type=Path, nargs="?")
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--cases", type=int, default=20)
    parser.add_argument("--steps", type=int, default=128)
    parser.add_argument("--timeout", type=positive, default=10)
    parser.add_argument("--max-latency-ms", type=positive, default=250)
    parser.add_argument("--memory-mib", type=int, default=4096)
    parser.add_argument("--confirm", type=int, default=2)
    parser.add_argument("--max-attempts", type=int, default=128)
    args = parser.parse_args()
    args.binary = args.binary.resolve()
    args.output = args.output.resolve()
    if not args.binary.is_file():
        parser.error("worker missing; run cargo build --features fuzzing --bin instantwm-fuzz")
    if not 0 <= args.steps <= 512 or args.cases < 1 or args.memory_mib < 128:
        parser.error("steps must be 0..512, cases positive, memory at least 128 MiB")
    if args.seed < 0 or args.seed + args.cases > 2**64 or args.confirm < 1 or args.max_attempts < args.confirm:
        parser.error("invalid seed or reduction limits")
    if args.mode != "campaign" and args.case is None:
        parser.error("replay/minimize requires a case file")
    args.output.mkdir(parents=True, exist_ok=False)
    revision = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    manifest = dict(revision=revision, platform=platform.platform(), binary=str(args.binary),
                    settings={k: str(v) if isinstance(v, Path) else v for k, v in vars(args).items()})
    (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    if args.mode == "minimize":
        return minimize(args, read_case(args.case))
    if args.mode == "replay":
        result = execute(args, read_case(args.case), args.output / "replay")
        print(json.dumps(result))
        return int(result["signature"] is not None)
    failures = []
    # Fixed regressions run before generated traces and are reusable by CI.
    cases = [(path.stem, read_case(path)) for path in sorted((ROOT / "tests/fuzz_corpus").glob("*.json"))]
    cases.extend((f"seed-{seed}", generated_case(args, seed)) for seed in range(args.seed, args.seed + args.cases))
    for name, case in cases:
        result = execute(args, case, args.output / name)
        if result["signature"]:
            failures.append(name)
        print(f"{name}: {result['signature'] or 'PASS'} ({result['elapsed_seconds']:.3f}s)", flush=True)
    (args.output / "summary.json").write_text(json.dumps(dict(cases=len(cases), failures=failures), indent=2) + "\n")
    return int(bool(failures))


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.SubprocessError) as error:
        print(f"fuzz runner: {error}", file=sys.stderr)
        sys.exit(2)
