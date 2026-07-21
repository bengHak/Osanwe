#!/bin/sh
set -eu

REPOSITORY=${OSANWE_REPOSITORY:-bengHak/Osanwe}
VERSION=${OSANWE_VERSION:-latest}
INSTALL_DIR=${OSANWE_INSTALL_DIR:-"${HOME:?HOME is required}/.local/bin"}
TOKEN=${OSANWE_GITHUB_TOKEN:-${GH_TOKEN:-}}

usage() {
  cat <<'USAGE'
Install Osanwe from a GitHub release.

Usage:
  install.sh
  install.sh --print-target
  install.sh --help

Environment:
  OSANWE_VERSION         Release tag, for example v0.1.0 (default: latest)
  OSANWE_INSTALL_DIR     Destination directory (default: ~/.local/bin)
  OSANWE_GITHUB_TOKEN    GitHub token for private repositories
  GH_TOKEN               Fallback GitHub token
  OSANWE_REPOSITORY      owner/repository override
  OSANWE_OS              linux or darwin override
  OSANWE_ARCH            x86_64 or aarch64 override
  OSANWE_DOWNLOAD_BASE   Release download mirror override
USAGE
}

die() {
  printf 'osanwe installer: %s\n' "$*" >&2
  exit 1
}

need() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

normalize_os() {
  case "$1" in
    Linux|linux) printf '%s\n' linux ;;
    Darwin|darwin|macOS|macos) printf '%s\n' darwin ;;
    *) die "unsupported operating system: $1" ;;
  esac
}

normalize_arch() {
  case "$1" in
    x86_64|amd64) printf '%s\n' x86_64 ;;
    aarch64|arm64) printf '%s\n' aarch64 ;;
    *) die "unsupported architecture: $1" ;;
  esac
}

detect_target() {
  os=$(normalize_os "${OSANWE_OS:-$(uname -s)}")
  arch=$(normalize_arch "${OSANWE_ARCH:-$(uname -m)}")
  case "$os" in
    linux) printf '%s-unknown-linux-gnu\n' "$arch" ;;
    darwin) printf '%s-apple-darwin\n' "$arch" ;;
  esac
}

curl_common() {
  if [ -n "$TOKEN" ]; then
    curl --fail --silent --show-error --location \
      --retry 3 --retry-delay 1 \
      --header "Authorization: Bearer $TOKEN" \
      --header "X-GitHub-Api-Version: 2022-11-28" \
      "$@"
  else
    curl --fail --silent --show-error --location \
      --retry 3 --retry-delay 1 \
      "$@"
  fi
}

release_base() {
  if [ -n "${OSANWE_DOWNLOAD_BASE:-}" ]; then
    printf '%s\n' "${OSANWE_DOWNLOAD_BASE%/}"
  elif [ "$VERSION" = latest ]; then
    printf 'https://github.com/%s/releases/latest/download\n' "$REPOSITORY"
  else
    case "$VERSION" in
      v*) tag=$VERSION ;;
      *) tag=v$VERSION ;;
    esac
    printf 'https://github.com/%s/releases/download/%s\n' "$REPOSITORY" "$tag"
  fi
}

parse_asset_api_url() {
  json_file=$1
  asset_name=$2
  if command -v python3 >/dev/null 2>&1; then
    python3 - "$json_file" "$asset_name" <<'PY'
import json
import pathlib
import sys

release = json.loads(pathlib.Path(sys.argv[1]).read_text())
name = sys.argv[2]
for asset in release.get("assets", []):
    if asset.get("name") == name:
        print(asset["url"])
        break
else:
    raise SystemExit(f"release asset not found: {name}")
PY
  elif command -v jq >/dev/null 2>&1; then
    jq -er --arg name "$asset_name" '.assets[] | select(.name == $name) | .url' "$json_file"
  else
    die "private release fallback requires python3 or jq"
  fi
}

download_api_asset() {
  asset_name=$1
  destination=$2
  release_json=$3
  api_base=https://api.github.com/repos/$REPOSITORY/releases
  if [ "$VERSION" = latest ]; then
    release_url=$api_base/latest
  else
    case "$VERSION" in
      v*) tag=$VERSION ;;
      *) tag=v$VERSION ;;
    esac
    release_url=$api_base/tags/$tag
  fi

  curl_common --header "Accept: application/vnd.github+json" \
    --output "$release_json" "$release_url"
  asset_url=$(parse_asset_api_url "$release_json" "$asset_name")
  curl_common --header "Accept: application/octet-stream" \
    --output "$destination" "$asset_url"
}

download_asset() {
  asset_name=$1
  destination=$2
  release_json=$3
  base=$(release_base)
  if curl_common --output "$destination" "$base/$asset_name"; then
    return 0
  fi
  [ -n "$TOKEN" ] || die "failed to download $asset_name; set OSANWE_GITHUB_TOKEN for a private repository"
  download_api_asset "$asset_name" "$destination" "$release_json"
}

verify_checksum() {
  archive=$1
  sums=$2
  asset_name=$3
  expected=$(awk -v name="$asset_name" '
    $2 == name || $2 == "*" name { print $1; exit }
  ' "$sums")
  [ -n "$expected" ] || die "checksum for $asset_name is missing"

  if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$archive" | awk '{print $1}')
  elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$archive" | awk '{print $1}')
  else
    die "sha256sum or shasum is required"
  fi
  [ "$actual" = "$expected" ] || die "checksum mismatch for $asset_name"
}

main() {
  case "${1:-}" in
    --help|-h)
      usage
      exit 0
      ;;
    --print-target)
      detect_target
      exit 0
      ;;
    '') ;;
    *) die "unknown argument: $1" ;;
  esac

  need curl
  need tar
  target=$(detect_target)
  asset=osanwe-$target.tar.gz
  temporary=$(mktemp -d "${TMPDIR:-/tmp}/osanwe-install.XXXXXX")
  trap 'rm -rf "$temporary"' EXIT HUP INT TERM

  archive=$temporary/$asset
  sums=$temporary/SHA256SUMS
  release_json=$temporary/release.json
  download_asset "$asset" "$archive" "$release_json"
  download_asset SHA256SUMS "$sums" "$release_json"
  verify_checksum "$archive" "$sums" "$asset"

  extract=$temporary/extract
  mkdir -p "$extract"
  tar -xzf "$archive" -C "$extract"
  [ -f "$extract/osanwe" ] || die "release archive does not contain osanwe"

  mkdir -p "$INSTALL_DIR"
  destination=$INSTALL_DIR/osanwe
  staging=$INSTALL_DIR/.osanwe-install-$$
  cp "$extract/osanwe" "$staging"
  chmod 755 "$staging"
  mv -f "$staging" "$destination"

  printf 'Installed Osanwe to %s\n' "$destination"
  case :$PATH: in
    *:"$INSTALL_DIR":*) ;;
    *) printf 'Add %s to PATH to run osanwe from any directory.\n' "$INSTALL_DIR" ;;
  esac
}

main "$@"
