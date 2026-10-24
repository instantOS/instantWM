#!/bin/sh
# Build a native Alpine package from the current source tree.
# Use APK_BUILD_PROFILE=dev for local validation; CI uses the release profile.
set -eu

ROOT_DIR=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT_DIR"
for command in abuild apk tar sha256sum; do
	command -v "$command" >/dev/null 2>&1 || {
		printf 'missing dependency: %s (run inside Alpine with abuild)\n' "$command" >&2
		exit 1
	}
done
ARCH=$(apk --print-arch)
case "$ARCH" in
x86_64 | aarch64) ;;
*)
	printf 'unsupported apk architecture: %s\n' "$ARCH" >&2
	exit 1
	;;
esac
VERSION=$(awk -F '"' '/^version =/ {print $2; exit}' Cargo.toml)
NAME=$(awk -F '"' '/^name =/ {print $2; exit}' Cargo.toml)
[ -n "$VERSION" ] && [ -n "$NAME" ] || {
	printf 'failed to determine package name/version from Cargo.toml\n' >&2
	exit 1
}
WORK_DIR=$(mktemp -d)
trap 'rm -rf "$WORK_DIR"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
PACKAGE_DIR="$WORK_DIR/instantos/$NAME"
mkdir -p "$PACKAGE_DIR" "$ROOT_DIR/target/apk"
# Include uncommitted source changes for local builds, but omit generated output
# and caches. The archive lives outside the source tree to avoid including itself.
tar --exclude='./.git' --exclude='./target' --exclude='./artifacts' \
	--exclude='./dist' --exclude='./sysroot' --exclude='./.cache' \
	-czf "$PACKAGE_DIR/source.tar.gz" .
sed "s/@VERSION@/$VERSION/g" packaging/alpine/APKBUILD.in >"$PACKAGE_DIR/APKBUILD"
(
	cd "$PACKAGE_DIR"
	abuild -F checksum
	abuild -F -r -P "$WORK_DIR/packages"
)
OUTPUT="$ROOT_DIR/target/apk/${NAME}-${VERSION}-r0-${ARCH}.apk"
install -m644 "$WORK_DIR/packages/instantos/$ARCH/${NAME}-${VERSION}-r0.apk" "$OUTPUT"
(cd "$ROOT_DIR/target/apk" && sha256sum "${OUTPUT##*/}" >"${OUTPUT##*/}.sha256")
printf 'apk package created at %s\n' "$OUTPUT"
