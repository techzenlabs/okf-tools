#!/usr/bin/env bash
# Put nix on the runner, if the runner does not already have it.
#
# The default hosted runners ship an ordinary Ubuntu image with no nix in it,
# and every later step of the job is a nix invocation. The shape is the
# script bsi already runs on its Gitea runner's plain-Ubuntu image, which
# took it from natbengroup/data-warehouse's ci_nix_setup.sh with the attic
# cache half removed. The root branch below is for container runners that
# start as root; a hosted runner starts as an ordinary user and the
# installer reaches for the runner's passwordless sudo instead.
#
# Nothing here holds a forge credential.
set -euo pipefail

if command -v nix >/dev/null 2>&1; then
  nix --version
  echo "install-nix: the runner already has nix."
  exit 0
fi

# The single-user installer wants the build users to exist when it runs as
# root, and a container image does not ship them.
if [ "$(id -u)" = "0" ] && ! getent group nixbld >/dev/null 2>&1; then
  groupadd -r nixbld
  nologin=$(command -v nologin || printf '/usr/sbin/nologin')
  for index in $(seq 1 10); do
    useradd -r -g nixbld -G nixbld -M -N -s "$nologin" "nixbld${index}"
  done
fi

installer=$(mktemp)
curl --fail --location --silent --show-error https://nixos.org/nix/install --output "$installer"
sh "$installer" --no-daemon
rm -f "$installer"

# shellcheck disable=SC1091
. "$HOME/.nix-profile/etc/profile.d/nix.sh"

if [ -n "${GITHUB_PATH:-}" ]; then
  printf '%s\n' "$HOME/.nix-profile/bin" >>"$GITHUB_PATH"
fi

mkdir -p "$HOME/.config/nix"
printf 'experimental-features = nix-command flakes\n' >>"$HOME/.config/nix/nix.conf"

nix --version
echo "install-nix: nix installed for this job."
