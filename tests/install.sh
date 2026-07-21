#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TMP_ROOT=${TMPDIR:-/tmp}/osanwe-install-test-$$
trap 'rm -rf "$TMP_ROOT"' EXIT HUP INT TERM
mkdir -p "$TMP_ROOT/fixture" "$TMP_ROOT/fake-bin" "$TMP_ROOT/home"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_file() {
  [ -f "$1" ] || fail "expected file: $1"
}

assert_contains() {
  haystack=$1
  needle=$2
  case "$haystack" in
    *"$needle"*) ;;
    *) fail "expected [$haystack] to contain [$needle]" ;;
  esac
}

make_fixture() {
  target=$1
  stage="$TMP_ROOT/stage-$target"
  mkdir -p "$stage"
  cat > "$stage/osanwe" <<'BIN'
#!/bin/sh
printf 'osanwe fixture\n'
BIN
  chmod +x "$stage/osanwe"
  cp "$ROOT/README.md" "$stage/README.md"
  cp "$ROOT/LICENSE" "$stage/LICENSE"

  asset="osanwe-$target.tar.gz"
  tar -czf "$TMP_ROOT/fixture/$asset" -C "$stage" osanwe README.md LICENSE
  (
    cd "$TMP_ROOT/fixture"
    sha256sum "$asset" > SHA256SUMS
  )
}

cat > "$TMP_ROOT/fake-bin/curl" <<'EOF_CURL'
#!/bin/sh
set -eu
output=
url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o|--output)
      output=$2
      shift 2
      ;;
    -H|--header)
      printf '%s\n' "$2" >> "${OSANWE_TEST_CURL_LOG:?}"
      shift 2
      ;;
    -* )
      shift
      ;;
    * )
      url=$1
      shift
      ;;
  esac
done
[ -n "$output" ] || { echo 'fake curl: missing output' >&2; exit 2; }
[ -n "$url" ] || { echo 'fake curl: missing URL' >&2; exit 2; }
name=${url##*/}
cp "${OSANWE_TEST_FIXTURE:?}/$name" "$output"
EOF_CURL
chmod +x "$TMP_ROOT/fake-bin/curl"

make_fixture x86_64-unknown-linux-gnu

TARGET=$(OSANWE_OS=linux OSANWE_ARCH=x86_64 sh "$ROOT/install.sh" --print-target)
[ "$TARGET" = "x86_64-unknown-linux-gnu" ] || fail "unexpected target: $TARGET"
DARWIN_TARGET=$(OSANWE_OS=Darwin OSANWE_ARCH=arm64 sh "$ROOT/install.sh" --print-target)
[ "$DARWIN_TARGET" = "aarch64-apple-darwin" ] || fail "unexpected Darwin target: $DARWIN_TARGET"

CURL_LOG="$TMP_ROOT/curl.log"
: > "$CURL_LOG"
HOME="$TMP_ROOT/home" \
PATH="$TMP_ROOT/fake-bin:$PATH" \
OSANWE_OS=linux \
OSANWE_ARCH=x86_64 \
OSANWE_VERSION=v0.1.0 \
OSANWE_INSTALL_DIR="$TMP_ROOT/home/bin" \
OSANWE_GITHUB_TOKEN=test-token \
OSANWE_TEST_CURL_LOG="$CURL_LOG" \
OSANWE_TEST_FIXTURE="$TMP_ROOT/fixture" \
sh "$ROOT/install.sh" >"$TMP_ROOT/install.out" 2>"$TMP_ROOT/install.err"

assert_file "$TMP_ROOT/home/bin/osanwe"
OUTPUT=$($TMP_ROOT/home/bin/osanwe)
[ "$OUTPUT" = "osanwe fixture" ] || fail "installed binary did not execute"
HEADERS=$(cat "$CURL_LOG")
assert_contains "$HEADERS" "Authorization: Bearer test-token"
INSTALL_OUT=$(cat "$TMP_ROOT/install.out")
assert_contains "$INSTALL_OUT" "Installed Osanwe"
assert_contains "$INSTALL_OUT" "Next steps"
assert_contains "$INSTALL_OUT" "Zellij 0.44+"
assert_contains "$INSTALL_OUT" "osanwe doctor"

if OSANWE_OS=plan9 OSANWE_ARCH=x86_64 sh "$ROOT/install.sh" --print-target >"$TMP_ROOT/unsupported.out" 2>"$TMP_ROOT/unsupported.err"; then
  fail "unsupported OS unexpectedly succeeded"
fi
assert_contains "$(cat "$TMP_ROOT/unsupported.err")" "unsupported operating system"

printf '%064d  %s\n' 0 osanwe-x86_64-unknown-linux-gnu.tar.gz > "$TMP_ROOT/fixture/SHA256SUMS"
if HOME="$TMP_ROOT/home" \
  PATH="$TMP_ROOT/fake-bin:$PATH" \
  OSANWE_OS=linux \
  OSANWE_ARCH=x86_64 \
  OSANWE_VERSION=v0.1.0 \
  OSANWE_INSTALL_DIR="$TMP_ROOT/bad-checksum-bin" \
  OSANWE_TEST_CURL_LOG="$CURL_LOG" \
  OSANWE_TEST_FIXTURE="$TMP_ROOT/fixture" \
  sh "$ROOT/install.sh" >"$TMP_ROOT/checksum.out" 2>"$TMP_ROOT/checksum.err"; then
  fail "checksum mismatch unexpectedly succeeded"
fi
assert_contains "$(cat "$TMP_ROOT/checksum.err")" "checksum mismatch"

printf 'install tests passed\n'
