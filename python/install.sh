#!/bin/sh
# Link disk-tidy into ~/.local/bin and create a config if none exists.
set -e
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$HOME/.local/bin" "$HOME/.config/disk-tidy"
ln -sf "$here/disk-tidy" "$HOME/.local/bin/disk-tidy"
[ -e "$HOME/.config/disk-tidy/config.toml" ] || cp "$here/config.example.toml" "$HOME/.config/disk-tidy/config.toml"
echo "Installed: $HOME/.local/bin/disk-tidy"
