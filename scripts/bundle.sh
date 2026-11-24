#!/bin/zsh
# Build the dev app bundle: target/Wax.app (debug build, icon, Info.plist).
# The bundle id stays dev.gpui-mail so notifications and saved state carry over.
set -e
cd "$(dirname "$0")/.."
profile=${1:-debug}
app=target/Wax.app
mkdir -p $app/Contents/MacOS $app/Contents/Resources
cp target/$profile/gpui-mail $app/Contents/MacOS/gpui-mail
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
<key>CFBundleVersion</key><string>0.1</string>
<key>CFBundleShortVersionString</key><string>0.1</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
touch $app
echo "built $app"
