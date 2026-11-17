#!/bin/sh
# Link docker-tidy into ~/.local/bin and create a config if none exists.
set -e
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$HOME/.local/bin" "$HOME/.config/docker-tidy"
chmod +x "$here/docker_tidy.py"
ln -sf "$here/docker_tidy.py" "$HOME/.local/bin/docker-tidy"
[ -e "$HOME/.config/docker-tidy/config.toml" ] || cp "$here/config.example.toml" "$HOME/.config/docker-tidy/config.toml"
echo "Installed: $HOME/.local/bin/docker-tidy"
