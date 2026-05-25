#!/usr/bin/env python3
import argparse
import datetime as dt
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]


def run(args: list[str]) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def read(path: str) -> str:
    return (ROOT / path).read_text()


def write(path: str, text: str) -> None:
    (ROOT / path).write_text(text)


def current_version() -> str:
    match = re.search(r'^version = "([^"]+)"', read("Cargo.toml"), re.MULTILINE)
    if not match:
        raise SystemExit("failed to find package version in Cargo.toml")
    return match.group(1)


def latest_version_tag() -> str | None:
    tags = run(
        ["git", "tag", "--merged", "HEAD", "--sort=-v:refname", "--list", "v[0-9]*"]
    )
    return tags.splitlines()[0] if tags else None


def commits_since(tag: str | None) -> list[tuple[str, str, str]]:
    git_range = f"{tag}..HEAD" if tag else "HEAD"
    raw = run(["git", "log", "--format=%H%x00%s%x00%b%x1e", git_range])
    commits = []
    for entry in raw.strip("\x1e").split("\x1e"):
        if not entry.strip():
            continue
        sha, subject, body = entry.lstrip("\n").split("\x00", 2)
        if subject.startswith(("chore: release v", "chore(release):")):
            continue
        commits.append((sha, subject.strip(), body.strip()))
    return commits


def bump_level(commits: list[tuple[str, str, str]], requested: str) -> str | None:
    if requested != "auto":
        return requested
    level: str | None = None
    for _, subject, body in commits:
        if "BREAKING CHANGE" in body or re.match(
            r"^[a-zA-Z]+(?:\([^)]+\))?!:", subject
        ):
            return "major"
        commit_type = subject.split(":", 1)[0].split("(", 1)[0].rstrip("!")
        if commit_type == "feat":
            level = "minor"
        elif (
            commit_type not in {"chore", "ci", "docs", "style", "test"}
            and level != "minor"
        ):
            level = "patch"
    return level


def bump_version(version: str, level: str) -> str:
    major, minor, patch = [int(part) for part in version.split(".")]
    if level == "major":
        return f"{major + 1}.0.0"
    if level == "minor":
        return f"{major}.{minor + 1}.0"
    if level == "patch":
        return f"{major}.{minor}.{patch + 1}"
    raise SystemExit(f"unknown bump level: {level}")


def update_cargo_toml(version: str) -> None:
    text = read("Cargo.toml")
    text = re.sub(
        r'^(version = )"[^"]+"', rf'\1"{version}"', text, count=1, flags=re.MULTILINE
    )
    write("Cargo.toml", text)


def update_cargo_lock(version: str) -> None:
    text = read("Cargo.lock")
    # This repository has one package today; fail loudly if that changes.
    pattern = re.compile(r'(\[\[package\]\]\nname = "instantwm"\nversion = )"[^"]+"')
    text, count = pattern.subn(rf'\1"{version}"', text, count=1)
    if count == 0:
        raise SystemExit("failed to find instantwm package in Cargo.lock")
    write("Cargo.lock", text)


def update_pkgbuild(version: str) -> None:
    path = ROOT / "packaging/arch/PKGBUILD"
    if not path.exists():
        return
    text = path.read_text()
    text = re.sub(r"^pkgver=.*", f"pkgver={version}", text, flags=re.MULTILINE)
    text = re.sub(r"^pkgrel=.*", "pkgrel=1", text, flags=re.MULTILINE)
    path.write_text(text)


def clean_subject(subject: str) -> tuple[str, str]:
    match = re.match(r"^(?P<type>[a-zA-Z]+)(?:\([^)]+\))?!?:\s*(?P<text>.+)$", subject)
    if not match:
        return "Other", subject
    commit_type = match.group("type")
    text = match.group("text")
    if commit_type == "feat":
        return "Added", text
    if commit_type == "fix":
        return "Fixed", text
    if commit_type in {"perf", "refactor"}:
        return "Changed", text
    return "Other", text


def update_changelog(
    version: str, previous_tag: str | None, commits: list[tuple[str, str, str]]
) -> None:
    text = read("CHANGELOG.md")
    if re.search(rf"^## \[{re.escape(version)}\]", text, re.MULTILINE):
        print(
            f"Version {version} already exists in CHANGELOG.md; leaving changelog unchanged."
        )
        return

    today = dt.date.today().isoformat()
    compare_base = previous_tag or "HEAD"
    header = f"## [{version}](https://github.com/instantOS/instantWM/compare/{compare_base}...v{version}) - {today}"
    groups: dict[str, list[str]] = {
        "Added": [],
        "Changed": [],
        "Fixed": [],
        "Other": [],
    }
    seen: set[str] = set()
    for _, subject, _ in reversed(commits):
        group, line = clean_subject(subject)
        if line in seen:
            continue
        seen.add(line)
        groups[group].append(f"- {line}")

    sections = [header]
    for group, lines in groups.items():
        if lines:
            sections.append(f"### {group}\n\n" + "\n".join(lines))
    release_notes = "\n\n".join(sections)

    marker = "## [Unreleased]"
    if marker not in text:
        raise SystemExit("failed to find Unreleased section in CHANGELOG.md")
    text = text.replace(marker, f"{marker}\n\n{release_notes}", 1)
    write("CHANGELOG.md", text)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--bump", choices=["auto", "patch", "minor", "major"], default="auto"
    )
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()

    old_version = current_version()
    previous_tag = latest_version_tag()
    commits = commits_since(previous_tag)
    level = bump_level(commits, args.bump)
    if not level:
        print("No release-worthy commits found; nothing to do.")
        return 0

    new_version = bump_version(old_version, level)
    print(f"Bumping {old_version} -> {new_version} ({level})")
    if args.dry_run:
        return 0

    update_cargo_toml(new_version)
    update_cargo_lock(new_version)
    update_pkgbuild(new_version)
    update_changelog(new_version, previous_tag, commits)
    return 0


if __name__ == "__main__":
    sys.exit(main())
