#!/usr/bin/env sh
set -eu

REPO="britonmearsty/hi"
BIN_NAME="hi"

say() { printf '%s\n' "[hi] $*"; }
fail() { printf '%s\n' "[hi] error: $*" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || fail "curl is required"

os=$(uname -s)
arch=$(uname -m)
case "$os:$arch" in
  Linux:x86_64) asset="hi-linux-x86_64"; archive="tar.gz" ;;
  Darwin:arm64) asset="hi-macos-aarch64"; archive="tar.gz" ;;
  *) fail "unsupported platform: $os $arch (supported: Linux x86_64 and macOS Apple Silicon)" ;;
esac

install_dir=""
if [ -w /usr/local/bin ]; then
  install_dir=/usr/local/bin
elif command -v sudo >/dev/null 2>&1 && sudo -n true >/dev/null 2>&1; then
  install_dir=/usr/local/bin
else
  install_dir="${HOME}/.local/bin"
  mkdir -p "$install_dir"
fi

tmp_dir=$(mktemp -d 2>/dev/null || mktemp -d -t hi-install)
cleanup() { rm -rf "$tmp_dir"; }
trap cleanup EXIT INT TERM

release_url=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" | sed -n "s#.*\"browser_download_url\": \"\([^\"]*${asset}[^\"]*\)\".*#\1#p" | head -n 1 || true)

if [ -n "$release_url" ]; then
  say "downloading latest $asset release"
  curl -fsSL "$release_url" -o "$tmp_dir/archive.$archive"
  mkdir -p "$tmp_dir/extracted"
  tar -xzf "$tmp_dir/archive.$archive" -C "$tmp_dir/extracted"
  binary=$(find "$tmp_dir/extracted" -type f -name "$BIN_NAME" -print -quit)
  [ -n "$binary" ] || fail "release archive did not contain the hi binary"
else
  command -v cargo >/dev/null 2>&1 || fail "no published release found and cargo is not installed"
  say "no published release found; installing from source with cargo"
  cargo install --git "https://github.com/$REPO.git" --locked --root "$tmp_dir/cargo-root"
  binary="$tmp_dir/cargo-root/bin/$BIN_NAME"
fi

if [ "$install_dir" = /usr/local/bin ] && [ ! -w "$install_dir" ]; then
  sudo install -m 0755 "$binary" "$install_dir/$BIN_NAME"
else
  install -m 0755 "$binary" "$install_dir/$BIN_NAME"
fi

say "installed $install_dir/$BIN_NAME"
if ! command -v hi >/dev/null 2>&1; then
  say "add this directory to your PATH:"
  say "export PATH=\"$install_dir:\$PATH\""
fi
say "run 'hi' to configure your provider and start chatting"
