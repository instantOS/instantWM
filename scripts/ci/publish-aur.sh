#!/usr/bin/env bash
set -euo pipefail

recipes=$(realpath "${1:?Usage: publish-aur.sh RECIPE_DIRECTORY}")
if [[ -z "${AUR_SSH_PRIVATE_KEY:-}" ]]; then
	echo 'Set the AUR_SSH_PRIVATE_KEY repository Actions secret to enable AUR publishing.' >&2
	exit 1
fi

umask 077
workdir=$(mktemp -d)
trap 'rm -rf -- "$workdir"' EXIT
printf '%s\n' "$AUR_SSH_PRIVATE_KEY" >"$workdir/key"
unset AUR_SSH_PRIVATE_KEY
# Host key fingerprint published by Arch Linux: SHA256:RFzBCUItH9LZS0cKB5UE6ceAYhBD5C8GeOBip8Z11+4
printf '%s\n' 'aur.archlinux.org ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEuBKrPzbawxA/k2g6NcyV5jmqwJ2s+zpgZGZ7tpLIcN' >"$workdir/known_hosts"
printf -v GIT_SSH_COMMAND 'ssh -F /dev/null -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=%q -i %q' "$workdir/known_hosts" "$workdir/key"
export GIT_SSH_COMMAND

package_version() {
	awk -F ' = ' '
		$1 ~ /^[[:space:]]+epoch$/ { epoch = $2 }
		$1 ~ /^[[:space:]]+pkgver$/ { version = $2 }
		$1 ~ /^[[:space:]]+pkgrel$/ { release = $2 }
		END {
			if (version == "" || release == "") exit 1
			print (epoch == "" ? "" : epoch ":") version "-" release
		}
	' "$1"
}

for package in instantwm instantwm-bin; do
	recipe="$recipes/$package"
	repo="$workdir/$package"
	if ! git clone --branch master "ssh://aur@aur.archlinux.org/$package.git" "$repo"; then
		echo "Could not clone $package from the AUR. Check that it has an initial submission and that SSH is available." >&2
		exit 1
	fi
	current=$(package_version "$repo/.SRCINFO")
	next=$(package_version "$recipe/.SRCINFO")
	if [[ "$(vercmp "$next" "$current")" == -1 ]]; then
		echo "Skipping $package $next because the AUR already has $current."
		continue
	fi

	cp -- "$recipe/PKGBUILD" "$recipe/.SRCINFO" "$repo/"
	git -C "$repo" add -- PKGBUILD .SRCINFO
	if git -C "$repo" diff --cached --quiet; then
		echo "$package $next is already published."
		continue
	fi

	git -C "$repo" -c user.name='github-actions[bot]' \
		-c user.email='41898282+github-actions[bot]@users.noreply.github.com' \
		commit -m "Update $package to $next"
	git -C "$repo" push origin HEAD:master
done
