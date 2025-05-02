#!/usr/bin/env bash

# A hacky script that copies the closure of the argument to the provided file
# binary cache, and then strips any paths from it that are already in
# cache.nixos.org. Saves quite a bit of space.

set -euo pipefail

CACHE_DIR="$1"
shift

if [ -z "$CACHE_DIR" ]; then
  echo "Must specify cache dir"
  exit 1
fi

# shellcheck disable=SC2046
nix-store -qR --include-outputs $(nix path-info --derivation "$@") \
  | nix copy --to "file://$CACHE_DIR?compression=zstd" --stdin

echo "Removing redundant paths..."

sed -n '/^StorePath:/ { s/^.*: *//; p }' $CACHE_DIR/*.narinfo \
  | nix path-info --store https://cache.nixos.org --json --stdin \
  | jq -r 'to_entries[] | select(.value != null or (.key|endswith(".drv"))) | .key' \
  | while read -r STORE_PATH; do

STORE_HASH=${STORE_PATH#/nix/store/}
STORE_HASH=${STORE_HASH%%-*}

URL=$(sed -n '/^URL:/ { s/^.*: *//; p }' "$CACHE_DIR/$STORE_HASH.narinfo")

rm "$CACHE_DIR/$STORE_HASH.narinfo" || true
rm "$CACHE_DIR/$URL" || true

done
