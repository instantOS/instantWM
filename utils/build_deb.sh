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

if ! command -v dpkg-shlibdeps >/dev/null 2>&1; then
	echo "dpkg-shlibdeps not found; install dpkg-dev to build the Debian package" >&2
	exit 1
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
install -Dm755 "scripts/startinstantos" "${PKG_DIR}/usr/bin/startinstantos"

install -Dm644 "LICENSE" "${PKG_DIR}/usr/share/doc/instantwm/copyright"
install -Dm644 "README.md" "${PKG_DIR}/usr/share/doc/instantwm/README.md"

# Install session files
install -Dm644 "utils/instantwm-x11.desktop" "${PKG_DIR}/usr/share/xsessions/instantwm.desktop"
install -Dm644 "utils/instantwm-wayland.desktop" "${PKG_DIR}/usr/share/wayland-sessions/instantwm-wayland.desktop"
install -Dm644 "utils/instantwm-wayland-debug.desktop" "${PKG_DIR}/usr/share/wayland-sessions/instantwm-debug.desktop"
install -Dm755 "scripts/instantwm-debug.sh" "${PKG_DIR}/usr/bin/instantwm-debug"
install -Dm644 "resources/instantwm-portals.conf" "${PKG_DIR}/usr/share/xdg-desktop-portal/instantwm-portals.conf"
install -Dm644 "resources/instantwm.portal" "${PKG_DIR}/usr/share/xdg-desktop-portal/portals/instantwm.portal"
install -Dm644 "resources/instantwm-session.target" "${PKG_DIR}/usr/lib/systemd/user/instantwm-session.target"

# Install man page if it exists
if [[ -f instantwm.1 ]]; then
	mkdir -p "${PKG_DIR}/usr/share/man/man1"
	install -Dm644 instantwm.1 "${PKG_DIR}/usr/share/man/man1/instantwm.1"
fi

# Runtime dependencies that dpkg-shlibdeps cannot see because they are
# dlopen()ed or invoked as commands instead of being linked normally.
RUNTIME_DEPS=(
	libxcb1             # x11rb loads libxcb.so.1 at runtime
	libwayland-server0  # Wayland compositor backend
	libwayland-client0  # nested Wayland backend (winit)
	libwayland-cursor0  # nested Wayland cursor handling
	libwayland-egl1     # nested Wayland EGL surfaces
	libegl1             # Smithay loads libEGL.so.1 at runtime
	libxkbcommon-x11-0  # winit X11 keymap support
	libxfixes3          # winit X11
	libxi6              # winit X11
	libx11-xcb1         # winit X11
	x11-xkb-utils       # X11 backend runs setxkbmap
	dbus                # session-bus helpers used by both backends
)

# dpkg-shlibdeps resolves the linked libraries from the build host's
# shlibs/symbols files. Build the package on the oldest supported release so
# the generated version constraints remain installable on newer ones.
SHLIBDEPS_DIR="${WORK_DIR}/shlibdeps"
rm -rf "${SHLIBDEPS_DIR}"
mkdir -p "${SHLIBDEPS_DIR}/debian"
cat > "${SHLIBDEPS_DIR}/debian/control" <<'EOF'
Source: instantwm
Section: x11
Priority: optional
Maintainer: instantOS <hello@instantos.io>

Package: instantwm
Architecture: any
Depends: ${shlibs:Depends}
Description: dependency probe
 dependency probe
EOF

SHLIBDEPS_LOG="${SHLIBDEPS_DIR}/dpkg-shlibdeps.log"
if ! SHLIBS_DEPS="$(
	cd "${SHLIBDEPS_DIR}"
	dpkg-shlibdeps -O \
		-e"${PKG_DIR}/usr/bin/instantwm" \
		-e"${PKG_DIR}/usr/bin/instantwmctl" 2>"${SHLIBDEPS_LOG}" |
		sed -n 's/^shlibs:Depends=//p'
)"; then
	cat "${SHLIBDEPS_LOG}" >&2
	echo "dpkg-shlibdeps failed" >&2
	exit 1
fi

if [[ -z "${SHLIBS_DEPS}" ]]; then
	echo "dpkg-shlibdeps did not produce any dependencies" >&2
	exit 1
fi

DEPENDS="${SHLIBS_DEPS}"
for dep in "${RUNTIME_DEPS[@]}"; do
	case " ${DEPENDS} " in
	*" ${dep} "* | *" ${dep} ("*) ;;
	*) DEPENDS="${DEPENDS}, ${dep}" ;;
	esac
done

# Generate control file from template
sed \
	-e "s/@VERSION@/${VERSION}/g" \
	-e "s/@ARCH@/${ARCH}/g" \
	-e "s|@DEPENDS@|${DEPENDS}|g" \
	"packaging/debian/control" > "${PKG_DIR}/DEBIAN/control"

OUTPUT="${WORK_DIR}/instantwm_${VERSION}_${ARCH}.deb"
dpkg-deb --build --root-owner-group "${PKG_DIR}" "${OUTPUT}"

echo "deb package created at ${OUTPUT}"
