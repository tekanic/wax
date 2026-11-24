#!/bin/zsh
# Build Wax in release mode and install it as ~/Applications/Wax.app.
# Quits a running Wax first and opens the new one afterwards.
set -e
cd "$(dirname "$0")/.."
cargo build --release
scripts/bundle.sh release
dest=${WAX_INSTALL_DIR:-$HOME/Applications}
mkdir -p "$dest"
osascript -e 'tell application id "dev.gpui-mail" to quit' >/dev/null 2>&1 || true
for i in 1 2 3 4 5; do pgrep -f "Wax.app/Contents/MacOS/gpui-mail" >/dev/null || break; sleep 1; done
pkill -f "Wax.app/Contents/MacOS/gpui-mail" 2>/dev/null || true
rm -rf "$dest/Wax.app"
ditto target/release/Wax.app "$dest/Wax.app"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$dest/Wax.app"
echo "installed $dest/Wax.app"
[ -n "$WAX_NO_OPEN" ] || open "$dest/Wax.app"
