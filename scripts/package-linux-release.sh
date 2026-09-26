#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux || "$(uname -m)" != x86_64 ]]; then
  echo "Linux x86_64 is required for this package" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

version="$(cargo +1.86.0 metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "pomodoro-tui"))')"
tag="${1:-v$version}"
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ || "$tag" != "v$version" ]]; then
  echo "tag $tag must match pomodoro-tui version v$version" >&2
  exit 1
fi

rustc +1.86.0 --version
cargo +1.86.0 --version
cargo +1.86.0 build --locked --release -p pomodoro-tui

archive_base="pomodoro-tui-${tag}-x86_64-unknown-linux-gnu"
output_dir="$repo_root/dist"
archive="$output_dir/$archive_base.tar.gz"
if [[ -e "$archive" || -e "$output_dir/SHA256SUMS" ]]; then
  echo "release output already exists in $output_dir" >&2
  exit 1
fi

mkdir -p "$output_dir"
stage_dir="$(mktemp -d)"
trap 'rm -rf -- "$stage_dir"' EXIT
mkdir "$stage_dir/$archive_base"
cp target/release/pomodoro-tui LICENSE README.md "$stage_dir/$archive_base/"
tar -C "$stage_dir" -czf "$archive" "$archive_base"
(cd "$output_dir" && sha256sum "$archive_base.tar.gz" > SHA256SUMS && sha256sum --check SHA256SUMS)

echo "created $archive and $output_dir/SHA256SUMS"
