#!/usr/bin/env bash
# Decide whether the current commit/ref should trigger a release.
#
# Writes the following keys to $GITHUB_OUTPUT (required):
#   version=<X.Y.Z>            -- from Cargo.toml
#   tag=v<X.Y.Z>               -- the corresponding git tag name
#   should_release=true|false  -- whether downstream jobs should run
#
# Behavior:
#   * On a tag push: the ref name must match v<version>; release.
#   * On `workflow_dispatch`: always release (creates the tag if needed).
#   * On a branch push: only release if HEAD is the bot's
#     `chore: release v<X.Y.Z>` commit. We accept that as either the head
#     commit's subject (squash/rebase merges, direct pushes) or, for a merge
#     commit, the subject of the merged-in branch tip (HEAD^2). This is the
#     part that broke for v0.1.3 when GitHub's default "Create a merge commit"
#     button put "Merge pull request #N ..." in the head subject.
#   * If the v<version> tag already exists, do nothing (idempotent reruns).
#
# Required environment:
#   GITHUB_OUTPUT, GITHUB_REF_TYPE, GITHUB_REF_NAME, GITHUB_EVENT_NAME
set -euo pipefail

# Defer version parsing to the shared script so there is exactly one place that
# knows how to read Cargo.toml. The subshell prevents extract-version.sh from
# also writing to $GITHUB_OUTPUT (we write our own keys explicitly below).
version="$(GITHUB_OUTPUT= bash "$(dirname "$0")/extract-version.sh")"

tag="v${version}"
{
  echo "version=${version}"
  echo "tag=${tag}"
} >> "$GITHUB_OUTPUT"

if [[ "${GITHUB_REF_TYPE}" == "tag" ]]; then
  if [[ "${GITHUB_REF_NAME}" != "$tag" ]]; then
    echo "Tag ${GITHUB_REF_NAME} does not match Cargo.toml version ${version}" >&2
    exit 1
  fi
  echo "should_release=true" >> "$GITHUB_OUTPUT"
  exit 0
fi

if [[ "${GITHUB_EVENT_NAME}" != "workflow_dispatch" ]]; then
  head_subject="$(git log -1 --format=%s)"
  release_subject="chore: release ${tag}"
  is_release_commit=false

  if [[ "${head_subject}" == "${release_subject}" ]]; then
    is_release_commit=true
  else
    # Merge commit: the PR tip lives at HEAD^2.
    parent_count="$(git rev-list --no-walk --parents HEAD | awk '{print NF - 1}')"
    if [[ "${parent_count}" -ge 2 ]]; then
      merged_subject="$(git log -1 --format=%s HEAD^2 2>/dev/null || true)"
      if [[ "${merged_subject}" == "${release_subject}" ]]; then
        is_release_commit=true
      fi
    fi
  fi

  if [[ "${is_release_commit}" != "true" ]]; then
    echo "Head commit is not a generated release commit (${head_subject}); skipping branch-triggered release."
    echo "should_release=false" >> "$GITHUB_OUTPUT"
    exit 0
  fi
fi

if git rev-parse -q --verify "refs/tags/${tag}" >/dev/null; then
  echo "Tag ${tag} already exists; skipping branch-triggered release."
  echo "should_release=false" >> "$GITHUB_OUTPUT"
  exit 0
fi

git tag -a "$tag" -m "Release ${tag}"
git push origin "$tag"

echo "Created ${tag}; publishing release from this workflow run."
echo "should_release=true" >> "$GITHUB_OUTPUT"
