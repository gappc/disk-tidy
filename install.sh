#!/bin/sh
# Build disk-tidy, install it to ~/.local/bin and create a config if none exists.
#   ./install.sh              build and install
#   ./install.sh --uninstall  remove the binary (the config stays)
set -e
here=$(cd "$(dirname "$0")" && pwd)
bin="$HOME/.local/bin/disk-tidy"
if [ "$1" = "--uninstall" ]; then
    rm -f "$bin"
    echo "Removed: $bin"
    exit 0
fi
cargo build --release --manifest-path "$here/Cargo.toml"
mkdir -p "$HOME/.local/bin" "$HOME/.config/disk-tidy"
rm -f "$bin" # may be the symlink of the Python version
install -m 755 "$here/target/release/disk-tidy" "$bin"
[ -e "$HOME/.config/disk-tidy/config.toml" ] || cp "$here/config.example.toml" "$HOME/.config/disk-tidy/config.toml"
echo "Installed: $bin"
