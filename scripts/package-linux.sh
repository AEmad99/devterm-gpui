#!/bin/sh
# Build a Linux package for DevTerm. The binary opens a local PTY with
# portable-pty. App id stays com.devterm.app.
set -eu
cd "$(dirname "$0")/.."
cargo build --release
stage="dist/linux/devterm"
rm -rf "$stage"
mkdir -p "$stage/node" "$stage/conpty"
cp target/release/devterm-gpui "$stage/devterm"
# The bundled agent is the Node runtime plus @earendil-works/pi-coding-agent.
# Drop a Node binary and the package into stage/node before shipping a release
# that must start the DevTerm agent offline.
if [ -d "${DEVTERM_NODE_DIR:-}" ]; then
  cp -a "$DEVTERM_NODE_DIR"/. "$stage/node/"
fi
cat > "$stage/devterm.desktop" <<'EOF'
[Desktop Entry]
Name=DevTerm
Exec=devterm
Icon=devterm
Type=Application
Categories=Utility;TerminalEmulator;
StartupWMClass=com.devterm.app
EOF
mkdir -p dist
tar -C dist/linux -czf dist/devterm-linux-x64.tar.gz devterm
echo "wrote dist/devterm-linux-x64.tar.gz"
