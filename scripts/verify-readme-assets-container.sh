#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
image=ccwrapped-readme-assets:chromium-148
binary=$repo_root/target/release/ccwrapped
mode=${1:---verify-render}

case "$mode" in
  --verify-render)
    mount_mode=ro
    user_args=()
    ;;
  --regenerate)
    mount_mode=rw
    user_args=(--user "$(id -u):$(id -g)" --env HOME=/tmp)
    ;;
  *)
    printf 'usage: %s [--verify-render|--regenerate]\n' "$0" >&2
    exit 2
    ;;
esac

if [[ ! -x $binary ]]; then
  printf 'Build the release binary first: cargo build --release --locked\n' >&2
  exit 1
fi

docker build \
  --network host \
  --file "$repo_root/scripts/readme-assets.Dockerfile" \
  --tag "$image" \
  "$repo_root"
docker run --rm --network none \
  "${user_args[@]}" \
  --volume "$repo_root:/work:$mount_mode" \
  --workdir /work \
  --env CHROMIUM_BIN=/usr/lib/chromium/chromium \
  --env CCWRAPPED_ASSET_BINARY=/work/target/release/ccwrapped \
  "$image" \
  scripts/generate-readme-assets.sh "$mode"
