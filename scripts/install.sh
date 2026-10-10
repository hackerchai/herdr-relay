#!/bin/sh
# Run only after checking the archive's checksum and its source.
set -eu
umask 077
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_dir=${HERDR_RELAY_INSTALL_DIR:-"$HOME/.local/bin"}
case "$install_dir" in /*) ;; *) echo 'Install directory must be absolute' >&2; exit 1 ;; esac
case "$(uname -s):$(uname -m):$(cat "$package_dir/TARGET")" in
  Darwin:arm64:aarch64-apple-darwin|Darwin:x86_64:x86_64-apple-darwin|Linux:aarch64:aarch64-unknown-linux-musl|Linux:x86_64:x86_64-unknown-linux-musl) ;;
  *) echo 'This archive does not match the current OS and architecture' >&2; exit 1 ;;
esac
(cd "$package_dir" && if command -v sha256sum >/dev/null 2>&1; then sha256sum -c SHA256SUMS; else shasum -a 256 -c SHA256SUMS; fi)
mkdir -p "$install_dir"
if [ -d "$install_dir/herdr-relay" ]; then
  echo 'Refusing to replace an existing herdr-relay directory' >&2
  exit 1
fi
install_tmp=$(mktemp "$install_dir/.herdr-relay.XXXXXXXX")
trap 'test -z "${install_tmp:-}" || rm -f "$install_tmp"' EXIT HUP INT TERM
cp "$package_dir/bin/herdr-relay" "$install_tmp"
chmod 755 "$install_tmp"
mv -f "$install_tmp" "$install_dir/herdr-relay"
install_tmp=''
printf 'Installed %s/herdr-relay\nAdd this directory to PATH if needed.\n' "$install_dir"
