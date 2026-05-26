#!/usr/bin/env bash
# Produce a release tarball (and its .sha256 sidecar) for a built binary set.
#
# Usage: package-binary.sh <target-triple> <version> <bin-dir>
#   target-triple   e.g. x86_64-unknown-linux-gnu
#   version         e.g. 0.1.3 (no leading 'v')
#   bin-dir         directory containing the already-built instantwm and
#                   instantwmctl binaries (e.g. target/release or
#                   target/<triple>/release)
#
# Outputs are written to ./artifacts/:
#   instantwm-<triple>-v<version>.tgz
#   instantwm-<triple>-v<version>.tgz.sha256
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <target-triple> <version> <bin-dir>" >&2
  exit 2
fi

triple="$1"
version="$2"
bin_dir="$3"

if [[ ! -x "${bin_dir}/instantwm" || ! -x "${bin_dir}/instantwmctl" ]]; then
  echo "Missing binaries in ${bin_dir}" >&2
  exit 1
fi

mkdir -p artifacts
pkg_dir="instantwm-${triple}-v${version}"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

mkdir -p "${tmpdir}/${pkg_dir}"
install -Dm0755 "${bin_dir}/instantwm"    "${tmpdir}/${pkg_dir}/instantwm"
install -Dm0755 "${bin_dir}/instantwmctl" "${tmpdir}/${pkg_dir}/instantwmctl"
tar -czf "artifacts/${pkg_dir}.tgz" -C "${tmpdir}" "${pkg_dir}"
sha256sum "artifacts/${pkg_dir}.tgz" > "artifacts/${pkg_dir}.tgz.sha256"
