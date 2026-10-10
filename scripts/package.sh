#!/bin/sh
# Build a versioned archive. Publication is a separate CI step.
set -eu
repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$repo_dir"
package_target=${1:?usage: scripts/package.sh TARGET}
case "$package_target" in
  aarch64-apple-darwin|x86_64-apple-darwin|aarch64-unknown-linux-musl|x86_64-unknown-linux-musl) ;;
  *) echo 'Unsupported package target' >&2; exit 1 ;;
esac
package_version=$(cargo metadata --format-version 1 --no-deps --locked | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
package_name="herdr-relay-$package_version-$package_target"
package_dir="$repo_dir/target/packages/$package_name"
mkdir -p "$package_dir/bin"
# Explicit build-input roots exclude profiles, Wrangler state and credentials.
# Hash before building and verify afterwards, so parallel edits cannot silently
# produce a candidate whose provenance describes a different source snapshot.
{
  printf '%s\n' Cargo.toml Cargo.lock relay/package.json relay/package-lock.json relay/tsconfig.json relay/wrangler.jsonc relay/wrangler.production.jsonc LICENSE
  find src tests protocol scripts patches .github relay/src relay/test relay/scripts -type f ! -name '*.pyc' ! -path '*/__pycache__/*'
} | LC_ALL=C sort | while IFS= read -r package_input; do
  shasum -a 256 "$package_input"
done > "$package_dir/SOURCE-MANIFEST.sha256"
cargo build --release --locked --target "$package_target"
if ! shasum -a 256 -c "$package_dir/SOURCE-MANIFEST.sha256" > /dev/null; then
  echo 'Build inputs changed during packaging; rebuild this candidate.' >&2
  exit 1
fi
{
  printf 'version=%s\ntarget=%s\nbuilt_utc=%s\n' "$package_version" "$package_target" "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf 'source_manifest_sha256=%s\n' "$(shasum -a 256 "$package_dir/SOURCE-MANIFEST.sha256" | cut -d ' ' -f 1)"
  printf 'git_commit=%s\n' "$(git rev-parse --verify HEAD 2>/dev/null || printf 'uncommitted-source-snapshot')"
  rustc -Vv
  cargo -V
  uname -srm
} > "$package_dir/BUILD-INFO.txt"
cp "target/$package_target/release/herdr-relay" "$package_dir/bin/herdr-relay"
chmod 755 "$package_dir/bin/herdr-relay"
cp scripts/install.sh "$package_dir/install.sh"
chmod 755 "$package_dir/install.sh"
cp docs/install.md "$package_dir/INSTALL.md"
printf '%s\n' "$package_target" > "$package_dir/TARGET"
printf '%s\n' 'MIT-licensed Herdr Relay. See BUILD-INFO.txt for source identity and documented compatibility limits.' > "$package_dir/CANDIDATE.txt"
if [ -f LICENSE ]; then cp LICENSE "$package_dir/LICENSE"; fi
(cd "$package_dir" && shasum -a 256 bin/herdr-relay install.sh INSTALL.md TARGET CANDIDATE.txt SOURCE-MANIFEST.sha256 BUILD-INFO.txt > SHA256SUMS && if [ -f LICENSE ]; then shasum -a 256 LICENSE >> SHA256SUMS; fi)
(cd "$repo_dir/target/packages" && tar -czf "$package_name.tar.gz" "$package_name" && shasum -a 256 "$package_name.tar.gz" > "$package_name.tar.gz.sha256")
printf '%s\n' "$repo_dir/target/packages/$package_name.tar.gz"
