#!/bin/bash
# Builds Chessie.app: one thing to double-click, with nothing to install.
#
# The engine and the piece model are copied inside the bundle, so the app does
# not depend on Homebrew or on a folder of assets sitting next to it.
set -euo pipefail

cd "$(dirname "$0")/.."
APP="dist/Chessie.app"
CONTENTS="$APP/Contents"

# The engine to ship. Override with ENGINE=/path/to/stockfish.
ENGINE="${ENGINE:-$(command -v stockfish || true)}"
if [ -z "$ENGINE" ] || [ ! -f "$ENGINE" ]; then
  echo "error: no stockfish binary to bundle." >&2
  echo "  install one (brew install stockfish) or set ENGINE=/path/to/stockfish" >&2
  exit 1
fi
# Follow symlinks; Homebrew's bin entry is usually one.
ENGINE="$(python3 -c 'import os,sys; print(os.path.realpath(sys.argv[1]))' "$ENGINE")"

echo "==> building"
cargo build --release

echo "==> assembling $APP"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"

cp target/release/chessie "$CONTENTS/MacOS/chessie"
cp "$ENGINE" "$CONTENTS/Resources/stockfish"
chmod +x "$CONTENTS/Resources/stockfish"
cp -R assets "$CONTENTS/Resources/assets"

# Stockfish is GPL-3; its licence and authors travel with the binary.
cp CREDITS.md "$CONTENTS/Resources/CREDITS.md"
for f in Copying.txt AUTHORS; do
  src="$(dirname "$(dirname "$ENGINE")")/$f"
  [ -f "$src" ] && cp "$src" "$CONTENTS/Resources/stockfish-$f" || true
done

cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Chessie</string>
  <key>CFBundleDisplayName</key><string>Chessie</string>
  <key>CFBundleExecutable</key><string>chessie</string>
  <key>CFBundleIdentifier</key><string>dev.ahmadmasud.chessie</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc signature. Not notarised — see the README on first launch.
codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || \
  echo "    (could not sign; the app will still run locally)"

echo "==> done"
du -sh "$APP" | sed 's/^/    /'
echo "    open $APP"
