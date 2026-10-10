#!/bin/sh
# Build a traceable custom server; never replaces or starts installed Herdr.
set -eu
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
revision=7b116c05bfda646af39d2524c54e70c751f57ee8
archive_sha=a8d914d60ae728deec9902ec29d706d305ba51c731424d6ffbf2a8af0b170124
source_dir="$repo_dir/target/upstream/herdr"
patch_file="$repo_dir/patches/herdr/0.9.3-control-queue.patch"
patch_sha=$(shasum -a 256 "$patch_file" | cut -d ' ' -f 1)
mode=${1:-build}
target=${2:-$(rustc -vV | sed -n 's/^host: //p')}
case "$mode" in prepare|test|build|package) ;; *) echo 'usage: scripts/build-herdr.sh [prepare|test|build|package] [TARGET]' >&2; exit 1 ;; esac
mkdir -p "$repo_dir/target/upstream"
if [ ! -f "$source_dir/.relay-patch-sha" ]; then
  if [ -d "$source_dir" ]; then
    echo 'Existing upstream source has no patch provenance; move it aside before preparing.' >&2
    exit 1
  fi
  archive="$repo_dir/target/upstream/herdr.source.tar.gz"
  if [ ! -f "$archive" ]; then
    curl --fail --location --retry 3 "https://codeload.github.com/herdrdev/herdr/tar.gz/$revision" -o "$archive"
  fi
  printf '%s  %s\n' "$archive_sha" "$archive" | shasum -a 256 -c
  mkdir "$source_dir"
  tar -xzf "$archive" -C "$source_dir" --strip-components=1
  (cd "$source_dir" && git apply --check "$patch_file" && git apply "$patch_file")
  printf '%s\n' "$patch_sha" > "$source_dir/.relay-patch-sha"
fi
if [ "$(cat "$source_dir/.relay-patch-sha")" != "$patch_sha" ]; then
  echo 'Patch changed; move the old upstream source aside and prepare again.' >&2
  exit 1
fi
cd "$source_dir"
case "$mode" in
  prepare) exit 0 ;;
  test) cargo test --locked -p herdr --bin herdr server::client_transport::tests; exit 0 ;;
esac
HERDR_BUILD_CHANNEL=custom HERDR_BUILD_ID=relay-control-bound-1 HERDR_BUILD_COMMIT="$revision" \
  cargo build --release --locked -p herdr --bin herdr --target "$target"
if [ "$mode" = package ]; then
  name="herdr-bounded-0.9.3-$target"
  output="$repo_dir/target/packages/$name"
  mkdir -p "$output/bin"
  cp "target/$target/release/herdr" "$output/bin/herdr"
  cp LICENSE "$output/LICENSE-APACHE"
  cp "$patch_file" "$output/control-queue.patch"
  cp "$repo_dir/patches/herdr/README.md" "$output/README.md"
  printf 'upstream_revision=%s\npatch_sha256=%s\ntarget=%s\n' "$revision" "$patch_sha" "$target" > "$output/BUILD-INFO.txt"
  (cd "$output" && shasum -a 256 bin/herdr LICENSE-APACHE control-queue.patch README.md BUILD-INFO.txt > SHA256SUMS)
  (cd "$repo_dir/target/packages" && tar -czf "$name.tar.gz" "$name" && shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256")
fi
