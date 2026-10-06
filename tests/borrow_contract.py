#!/usr/bin/env python3
"""Check rejected programs against the actual private backend API (debug check)."""
import json
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
result = subprocess.run(
    ["cargo", "rustc", "--locked", "--lib", "--message-format=json", "--",
     "--cfg", "instantwm_borrow_contract", "--check-cfg", "cfg(instantwm_borrow_contract)"],
    cwd=root, text=True, capture_output=True,
)
errors = []
for line in result.stdout.splitlines():
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    if message.get("reason") == "compiler-message" and message["message"]["level"] == "error":
        errors.append(message["message"])
expected = {"E0499": 2, "E0502": 1, "E0599": 1, "E0596": 1}
actual = {}
for error in errors:
    code = (error.get("code") or {}).get("code")
    actual[code] = actual.get(code, 0) + 1
    assert any(span["file_name"].endswith("tests/ui/native_borrow.rs")
               and span["is_primary"] for span in error["spans"]), error["rendered"]
assert result.returncode != 0 and actual == expected, (
    f"Unexpected compile result: {actual}\n{result.stderr}\n"
    + "\n".join(error["rendered"] for error in errors)
)
print("PASS: exclusive state access, non-reentrant effects, read/write separation, "
      "typed backend identity, and mutable shared effects")
