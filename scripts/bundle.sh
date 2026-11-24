#!/bin/zsh
# Make the app bundle from a build: scripts/bundle.sh [debug|release]
# Output: target/<profile>/Wax.app. Bundle id stays dev.gpui-mail so
# notification permission and saved state carry over.
set -e
cd "$(dirname "$0")/.."
profile=${1:-debug}
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
app=target/$profile/Wax.app
rm -rf $app
mkdir -p $app/Contents/MacOS $app/Contents/Resources
cp target/$profile/gpui-mail $app/Contents/MacOS/gpui-mail
[ "$profile" = release ] && strip -x $app/Contents/MacOS/gpui-mail
cp assets/brand/Wax.icns $app/Contents/Resources/Wax.icns
cat > $app/Contents/Info.plist <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>dev.gpui-mail</string>
<key>CFBundleName</key><string>Wax</string>
<key>CFBundleDisplayName</key><string>Wax</string>
<key>CFBundleExecutable</key><string>gpui-mail</string>
<key>CFBundleIconFile</key><string>Wax</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSHumanReadableCopyright</key><string>© 2026 Tekanics LLC</string>
</dict></plist>
PLIST
# Sign it so macOS keeps one identity for it across builds:
# WAX_SIGN_IDENTITY if set, else an "Apple Development" certificate whose
# name contains WAX_SIGN_MATCH (default "tekanics"), else the first
# "Apple Development" one, else ad hoc.
certs=$(security find-identity -v -p codesigning 2>/dev/null | grep "Apple Development" || true)
match=${WAX_SIGN_MATCH:-tekanics}
identity=${WAX_SIGN_IDENTITY:-$(print -r -- "$certs" | grep -i "$match" | awk '{print $2; exit}')}
[ -n "$identity" ] || identity=$(print -r -- "$certs" | awk '{print $2; exit}')
codesign --force --sign "${identity:--}" $app >/dev/null 2>&1 || codesign --force --sign - $app
echo "built $app"
