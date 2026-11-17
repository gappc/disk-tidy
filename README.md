# docker-tidy

Shows what Docker is taking up on disk, sorted by size, and removes only the parts that are safe to lose.

Docker has no dry run for `prune`. `docker-tidy preview` fills that gap: it reads `docker system df -v`
and groups everything by the command that would delete it.

## Usage

```sh
docker-tidy                 # same as: docker-tidy preview
docker-tidy preview --top 0 # list every entry, not just the top 25 per category
docker-tidy clean           # remove the [clean] categories, after asking
docker-tidy clean --yes     # without asking
```

`clean` removes only:

| Category | How |
|---|---|
| Dangling images (`<none>`) | `docker rmi <id>...` for exactly the listed IDs |
| Unused build cache | `docker builder prune -a -f` |
| Anonymous volumes without a container | `docker volume rm <id>...` for exactly the listed IDs |
| Named volumes matching `cache_volumes` | `docker volume rm <name>...`, never if they match `protect_volumes` |

Everything else (stopped containers, named images, other named volumes) is only listed, with the
command to remove it by hand.

## Install

```sh
./install.sh   # symlink to ~/.local/bin/docker-tidy, create ~/.config/docker-tidy/config.toml
```

Requires Python 3.11+ (uses `tomllib`), no other dependencies.

## Config

`~/.config/docker-tidy/config.toml`, see `config.example.toml`. Without a config, no named volume is
ever removed and anything matching `*db*` or `*data*` is shown as protected.

## Tests

```sh
python3 -m unittest discover -s tests
```
