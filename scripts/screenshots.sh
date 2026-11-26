#!/bin/zsh
set -euo pipefail
cd "$(dirname "$0")/.."
for t in "$HOME"/.rustup/toolchains/stable-*/bin; do
  [ -x "$t/cargo" ] && PATH="$t:$PATH" && export PATH
done
command -v cwebp >/dev/null || { echo "screenshots.sh needs cwebp (brew install webp)"; exit 1; }
pos=${WAX_SHOT_POS:-80 80}
home=$(mktemp -d /tmp/wax-demo.XXXX)
work=$(mktemp -d)
trap 'rm -rf "$home" "$work"' EXIT

cargo build -q
scripts/build-plugins.sh >/dev/null
bin=target/debug/gpui-mail
env HOME="$home" WAX_DEMO=1 WAX_DEMO_PLUGINS="$HOME/.config/wax/plugins" "$bin" --make-demo >/dev/null

cat > "$work/winid.swift" <<'EOF'
import CoreGraphics
import Foundation
let pid = Int32(CommandLine.arguments[1])!
let title = CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : nil
let list = CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as! [[String: Any]]
var best: (Int, Double)? = nil
for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    let b = w[kCGWindowBounds as String] as? [String: Any]
    let area = ((b?["Height"] as? Double) ?? 0) * ((b?["Width"] as? Double) ?? 0)
    if let t = title, (w[kCGWindowName as String] as? String) != t { continue }
    if area > (best?.1 ?? 0) { best = ((w[kCGWindowNumber as String] as? Int) ?? 0, area) }
}
if let b = best { print(b.0) } else { exit(1) }
EOF
swiftc -O "$work/winid.swift" -o "$work/winid"

shoot() {
  local view=$1 size=${2:-1440 900}
  echo "windowed $pos $size" > "$home/Library/Application Support/gpui-mail/window.txt"
  env HOME="$home" WAX_DEMO=1 WAX_DEMO_VIEW=$view "$bin" >/dev/null 2>&1 &
  local pid=$!
  sleep 7
  local id
  id=$("$work/winid" $pid)
  screencapture -x -o -l$id "$work/$view.png"
  kill $pid; wait $pid 2>/dev/null || true
  sips -Z 2200 "$work/$view.png" --out "$work/$view-web.png" >/dev/null
  cwebp -quiet -q 86 -m 6 "$work/$view-web.png" -o "docs/shots/${3:-$view}.webp"
  echo "docs/shots/${3:-$view}.webp"
}

mkdir -p docs/shots
for v in inbox light phishing compose fill tables signature sweep plugins newsletter; do shoot $v; done

echo "windowed $pos 1440 900" > "$home/Library/Application Support/gpui-mail/window.txt"
env HOME="$home" WAX_DEMO=1 WAX_DEMO_VIEW=about "$bin" >/dev/null 2>&1 &
pid=$!
sleep 7
screencapture -x -l$("$work/winid" $pid "About Wax") "$work/about.png"
kill $pid; wait $pid 2>/dev/null || true
cwebp -quiet -q 90 -m 6 "$work/about.png" -o docs/shots/about.webp
echo "docs/shots/about.webp"
shoot unified "960 700" compact
sips -c 1508 2880 --cropOffset 0 0 "$work/inbox.png" --out "$work/og.png" >/dev/null
sips -z 630 1200 "$work/og.png" --out "$work/og-small.png" >/dev/null
sips -s format jpeg -s formatOptions 85 "$work/og-small.png" --out docs/shots/og.jpg >/dev/null
echo "docs/shots/og.jpg"
