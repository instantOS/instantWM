#!/usr/bin/env bash
set -euo pipefail

# Build and install instantWM (Rust) with display manager sessions

PREFIX="${PREFIX:-/usr/local}"
DESTDIR="${DESTDIR:-}"

SUPERTOOL="sudo"
if [[ -x /usr/bin/doas ]] && [[ -s /etc/doas.conf ]]; then
    SUPERTOOL="doas"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "Building instantWM (release)..."
cargo build --release --manifest-path "$SCRIPT_DIR/Cargo.toml"

echo "Installing..."
$SUPERTOOL install -d "${DESTDIR}${PREFIX}/bin"
$SUPERTOOL install -d "${DESTDIR}/usr/share/xsessions"
$SUPERTOOL install -d "${DESTDIR}/usr/share/wayland-sessions"

# Binary
$SUPERTOOL install -m 755 "$SCRIPT_DIR/target/release/instantwm" "${DESTDIR}${PREFIX}/bin/instantwm-rs"

# instantwmctl helper binary
if [ -f "$SCRIPT_DIR/target/release/instantwmctl" ]; then
    $SUPERTOOL install -m 755 "$SCRIPT_DIR/target/release/instantwmctl" "${DESTDIR}${PREFIX}/bin/instantwmctl"
fi

# startinstantos session script
$SUPERTOOL install -m 755 "$SCRIPT_DIR/../startinstantos" "${DESTDIR}${PREFIX}/bin/startinstantos"

# X11 display manager session
$SUPERTOOL install -m 644 "$SCRIPT_DIR/instantwm-x11.desktop" "${DESTDIR}/usr/share/xsessions/instantwm.desktop"

# Wayland display manager session
$SUPERTOOL install -m 644 "$SCRIPT_DIR/instantwm-wayland.desktop" "${DESTDIR}/usr/share/wayland-sessions/instantwm.desktop"

echo "Done. instantWM installed to ${DESTDIR}${PREFIX}/bin/instantwm-rs"
echo "X11 session:    ${DESTDIR}/usr/share/xsessions/instantwm.desktop"
echo "Wayland session: ${DESTDIR}/usr/share/wayland-sessions/instantwm.desktop"
