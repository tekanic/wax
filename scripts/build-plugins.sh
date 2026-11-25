#!/bin/sh
# Build the plugins in plugins/ and install them into ~/.config/wax/plugins.
# A newly installed plugin starts switched off: turn it on in Settings → Plugins.
#   scripts/build-plugins.sh             build and install all (except test-*)
#   scripts/build-plugins.sh filter-rules
set -e
cd "$(dirname "$0")/.."
# Homebrew's Rust can shadow rustup's, which has the wasm32-wasip2 target.
for t in "$HOME"/.rustup/toolchains/stable-*/bin; do
  [ -x "$t/cargo" ] && PATH="$t:$PATH" && export PATH
  break
done
rustup target add wasm32-wasip2 >/dev/null 2>&1 || true
dest="${WAX_CONFIG_DIR:-$HOME/.config/wax}/plugins"
names="$*"
[ -n "$names" ] || names=$(ls plugins | grep -v '^test-')
for name in $names; do
  echo "Building $name"
  (cd "plugins/$name" && cargo build --release --target wasm32-wasip2 --quiet)
  wasm=$(ls "plugins/$name"/target/wasm32-wasip2/release/*.wasm | head -1)
  mkdir -p "$dest/$name"
  cp "$wasm" "$dest/$name/plugin.wasm"
  if [ ! -f "$dest/$name/config.toml" ] && [ -f "plugins/$name/config.example.toml" ]; then
    cp "plugins/$name/config.example.toml" "$dest/$name/config.toml"
  fi
  echo "  installed $dest/$name"
done
