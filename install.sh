#!/bin/sh
# Installs the allowit command.
#
#   curl -fsSL https://raw.githubusercontent.com/allowit-hq/allowit-cli/main/install.sh | sh
#
# 1. It installs a prebuilt binary from this repository's GitHub release when the release has
#    one for this system, and checks its SHA-256 against the release's SHA256SUMS file.
# 2. Otherwise it builds from source with cargo, at one exact commit, and says so. It installs
#    Rust with rustup when cargo is missing. As root on a system with apt-get it also installs
#    the missing build tools (git, a C compiler, CA certificates).
# It installs to ~/.local/bin and prints the version with the build revision.
#
# Settings (environment):
#   ALLOWIT_INSTALL_DIR  the directory of the binary (default: $HOME/.local/bin)
#   ALLOWIT_VERSION      a release tag such as v0.5.0 (default: the latest release)
#   ALLOWIT_RELEASE_URL  the base URL of the release files, for a mirror (default: GitHub)
#   ALLOWIT_REF          the branch, tag or commit of a source build (default: ALLOWIT_VERSION,
#                        or main without it)
#   ALLOWIT_FROM_SOURCE  1 builds from source even when a release binary exists
#   ALLOWIT_NO_DEPS      1 installs no build tool and no Rust; the script names what is missing
#   ALLOWIT_DRY_RUN      1 prints the plan and changes nothing
#   ALLOWIT_VERBOSE      1 prints the output of apt-get, rustup and cargo as they run
set -eu

REPO=allowit-hq/allowit-cli
SOURCE=https://github.com/$REPO
DIR=${ALLOWIT_INSTALL_DIR:-${HOME:?HOME is not set}/.local/bin}
# A source build of a requested release builds that release, never another revision.
REF=${ALLOWIT_REF:-${ALLOWIT_VERSION:-main}}
DRY=${ALLOWIT_DRY_RUN:-0}

say() { printf 'allowit install: %s\n' "$*" >&2; }
die() { printf 'allowit install: error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# The release target of this system, or nothing when no release serves it.
target() {
  os=$(uname -s) arch=$(uname -m)
  case $arch in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) return 0 ;;
  esac
  case $os in
    Linux) echo "$arch-unknown-linux-gnu" ;;
    Darwin) echo "$arch-apple-darwin" ;;
  esac
}

fetch() { # fetch URL FILE
  if have curl; then curl -fsSL --retry 2 -o "$2" "$1"
  elif have wget; then wget -q -O "$2" "$1"
  else return 1; fi
}

sha256() {
  if have sha256sum; then sha256sum "$1" | cut -d' ' -f1
  elif have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
  else return 1; fi
}

# Prints the commit that REF names in the source repository.
commit_of() {
  case $REF in
    *[!0-9a-f]* | "") ;;
    *) if [ ${#REF} = 40 ]; then echo "$REF"; return 0; fi ;;
  esac
  git ls-remote "$SOURCE.git" "refs/heads/$REF" "refs/tags/$REF^{}" "refs/tags/$REF" 2>/dev/null |
    sort -k2 -r | head -n 1 | cut -f1
}

TMP=$(mktemp -d 2>/dev/null || mktemp -d -t allowit-install)
trap 'rm -rf "$TMP"' EXIT INT TERM

# Runs a long tool quietly: its output goes to a log, and the end of the log is printed only
# when the tool fails. A reader, human or agent, sees one line per step.
quiet() {
  if [ "${ALLOWIT_VERBOSE:-0}" = 1 ]; then "$@" >&2 </dev/null; return; fi
  if "$@" >"$TMP/step.log" 2>&1 </dev/null; then return 0; fi
  tail -n 30 "$TMP/step.log" >&2
  return 1
}

install_binary() { # install_binary FILE
  [ "$DRY" = 1 ] && return 0
  mkdir -p "$DIR"
  cp "$1" "$DIR/allowit.new" && chmod 755 "$DIR/allowit.new" && mv "$DIR/allowit.new" "$DIR/allowit"
}

# 1. A release binary, when one exists for this system.
from_release() {
  [ "${ALLOWIT_FROM_SOURCE:-0}" != 1 ] || return 1
  triple=$(target)
  [ -n "$triple" ] || { say "no release binary is built for $(uname -s) $(uname -m)"; return 1; }
  have curl || have wget || return 1
  if [ -n "${ALLOWIT_RELEASE_URL:-}" ]; then base=$ALLOWIT_RELEASE_URL
  elif [ -n "${ALLOWIT_VERSION:-}" ]; then base=$SOURCE/releases/download/$ALLOWIT_VERSION
  else base=$SOURCE/releases/latest/download; fi
  asset=allowit-$triple.tar.gz
  fetch "$base/SHA256SUMS" "$TMP/SHA256SUMS" 2>/dev/null || { say "the release has no binaries"; return 1; }
  expected=$(awk -v f="$asset" '$2 == f || $2 == "*" f {print $1}' "$TMP/SHA256SUMS" | head -n 1)
  [ -n "$expected" ] || { say "the release has no binary for $triple"; return 1; }
  if [ "$DRY" = 1 ]; then say "would install the release binary $base/$asset to $DIR"; return 0; fi
  say "downloading the release binary $base/$asset"
  fetch "$base/$asset" "$TMP/$asset" || die "the download of $asset failed"
  actual=$(sha256 "$TMP/$asset") || die "no sha256sum or shasum to check the download"
  [ "$actual" = "$expected" ] || die "the SHA-256 of $asset is $actual; the release lists $expected. Nothing was installed."
  say "the SHA-256 matches the release: $actual"
  tar -xzf "$TMP/$asset" -C "$TMP" allowit || die "$asset holds no allowit binary"
  install_binary "$TMP/allowit"
}

# Missing build tools, installed only as root with apt-get, and named otherwise.
build_tools() {
  missing=
  have git || missing="$missing git"
  have cc || have gcc || have clang || missing="$missing build-essential"
  have curl || have wget || missing="$missing curl"
  [ -e /etc/ssl/certs/ca-certificates.crt ] || [ "$(uname -s)" != Linux ] || missing="$missing ca-certificates"
  [ -n "$missing" ] || return 0
  if [ "${ALLOWIT_NO_DEPS:-0}" != 1 ] && [ "$(id -u)" = 0 ] && have apt-get; then
    if [ "$DRY" = 1 ]; then say "would install with apt-get:$missing"; return 0; fi
    say "installing build tools with apt-get:$missing"
    # shellcheck disable=SC2086 # the list is split into package names on purpose
    DEBIAN_FRONTEND=noninteractive; export DEBIAN_FRONTEND
    quiet apt-get update -qq && quiet apt-get install -y -qq --no-install-recommends $missing ||
      die "apt-get could not install:$missing"
  else
    die "a source build needs:$missing. Install them, then run this script again."
  fi
}

# 2. A source build at one exact commit.
from_source() {
  say "no release binary is available; building from source at $SOURCE ($REF). This takes a few minutes."
  build_tools
  if ! have cargo && [ -x "$HOME/.cargo/bin/cargo" ]; then PATH=$HOME/.cargo/bin:$PATH; fi
  if ! have cargo; then
    [ "${ALLOWIT_NO_DEPS:-0}" != 1 ] || die "a source build needs cargo (Rust 1.98 or later): https://rustup.rs"
    if [ "$DRY" = 1 ]; then say "would install Rust with rustup (minimal profile)"
    else
      say "installing Rust with rustup (minimal profile) from https://sh.rustup.rs"
      fetch https://sh.rustup.rs "$TMP/rustup.sh" || die "the download of rustup failed"
      quiet sh "$TMP/rustup.sh" -y --profile minimal --default-toolchain stable || die "rustup failed"
      PATH=$HOME/.cargo/bin:$PATH
    fi
  fi
  if [ "$DRY" = 1 ]; then say "would build $SOURCE at $REF with cargo and install to $DIR"; return 0; fi
  commit=$(commit_of)
  [ -n "$commit" ] || die "$SOURCE has no branch, tag or commit $REF"
  say "building commit $commit with $(cargo --version)"
  # The build script records this revision in `allowit --version`.
  ALLOWIT_BUILD_REVISION=$(printf %.12s "$commit"); export ALLOWIT_BUILD_REVISION
  quiet cargo install --locked --git "$SOURCE.git" --rev "$commit" --bin allowit --root "$TMP/root" allowit-cli ||
    die "the build failed"
  install_binary "$TMP/root/bin/allowit"
}

from_release || from_source
[ "$DRY" != 1 ] || { say "dry run: nothing was changed"; exit 0; }

version=$("$DIR/allowit" --version) || die "$DIR/allowit does not run"
say "installed $version at $DIR/allowit"
case ":$PATH:" in
  *":$DIR:"*) ;;
  *)
    # A root shell in a container has no ~/.local/bin on PATH; /usr/local/bin is.
    if [ "$(id -u)" = 0 ] && [ -d /usr/local/bin ] && [ -w /usr/local/bin ] && [ "$DIR" != /usr/local/bin ]; then
      ln -sf "$DIR/allowit" /usr/local/bin/allowit && say "linked /usr/local/bin/allowit to it, because $DIR is not on PATH"
    else
      say "$DIR is not on PATH. Add it: export PATH=\"$DIR:\$PATH\""
    fi
    ;;
esac
say "next: run the line from the owner (allowit POLICY_ID help), or allowit guide"
echo "$version"
