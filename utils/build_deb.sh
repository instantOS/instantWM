#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

VERSION="$(awk -F '"' '/^version =/ {print $2; exit}' Cargo.toml)"
if [[ -z "${VERSION}" ]]; then
	echo "failed to extract version from Cargo.toml" >&2
	exit 1
fi

if [[ -n "${1:-}" ]]; then
	INSTANTWM_BIN="$1"
	INSTANTWMCTL_BIN="$2"
else
	INSTANTWM_BIN="target/release/instantwm"
	INSTANTWMCTL_BIN="target/release/instantwmctl"
fi

if [[ ! -x "${INSTANTWM_BIN}" ]]; then
	echo "binary not found or not executable: ${INSTANTWM_BIN}" >&2
	echo "build with 'cargo build --release' or pass binary paths as arguments" >&2
	exit 1
fi

if [[ ! -x "${INSTANTWMCTL_BIN}" ]]; then
	echo "binary not found or not executable: ${INSTANTWMCTL_BIN}" >&2
	echo "build with 'cargo build --release' or pass binary paths as arguments" >&2
	exit 1
fi

if command -v dpkg >/dev/null 2>&1; then
	ARCH="$(dpkg --print-architecture)"
else
	case "$(uname -m)" in
	x86_64) ARCH="amd64" ;;
	aarch64) ARCH="arm64" ;;
	armv7l) ARCH="armhf" ;;
	*) ARCH="$(uname -m)" ;;
	esac
fi

WORK_DIR="${ROOT_DIR}/target/deb"
PKG_DIR="${WORK_DIR}/instantwm_${VERSION}_${ARCH}"

rm -rf "${PKG_DIR}"
mkdir -p "${PKG_DIR}/DEBIAN" \
	"${PKG_DIR}/usr/bin" \
	"${PKG_DIR}/usr/share/doc/instantwm" \
	"${PKG_DIR}/usr/share/xdg-desktop-portal" \
	"${PKG_DIR}/usr/share/xsessions" \
	"${PKG_DIR}/usr/share/wayland-sessions"

install -Dm755 "${INSTANTWM_BIN}" "${PKG_DIR}/usr/bin/instantwm"
install -Dm755 "${INSTANTWMCTL_BIN}" "${PKG_DIR}/usr/bin/instantwmctl"

install -Dm644 "LICENSE" "${PKG_DIR}/usr/share/doc/instantwm/copyright"
install -Dm644 "README.md" "${PKG_DIR}/usr/share/doc/instantwm/README.md"

# Install session files
install -Dm644 "utils/instantwm-x11.desktop" "${PKG_DIR}/usr/share/xsessions/instantwm.desktop"
install -Dm644 "utils/instantwm-wayland.desktop" "${PKG_DIR}/usr/share/wayland-sessions/instantwm-wayland.desktop"
install -Dm644 "resources/instantwm-portals.conf" "${PKG_DIR}/usr/share/xdg-desktop-portal/instantwm-portals.conf"

# Install man page if it exists
if [[ -f instantwm.1 ]]; then
	mkdir -p "${PKG_DIR}/usr/share/man/man1"
	install -Dm644 instantwm.1 "${PKG_DIR}/usr/share/man/man1/instantwm.1"
fi

# Generate control file from template
sed "s/VERSION/${VERSION}/g" "packaging/debian/control" > "${PKG_DIR}/DEBIAN/control"

OUTPUT="${WORK_DIR}/instantwm_${VERSION}_${ARCH}.deb"
dpkg-deb --build --root-owner-group "${PKG_DIR}" "${OUTPUT}"

echo "deb package created at ${OUTPUT}"
