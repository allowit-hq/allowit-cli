#!/usr/bin/env bash
# Fetches the private dependency and the locked Cargo sources with a read-only key.
# The key lives in an agent that this script starts and stops. No later step can use it.
# This script prints fixed text only: the repository, its commit and the key never reach the log.
set -euo pipefail

fail() {
  echo "::error title=Private dependency::$1"
  exit 1
}

repository=${PRIVATE_DEPENDENCY_REPOSITORY:-}
manifest=${MANIFEST_PATH:-Cargo.toml}

if [[ -z "${PRIVATE_DEPENDENCY_KEY:-}" ]]; then
  fail "This run has no private dependency key. GitHub gives no repository secrets to pull requests from forks or from Dependabot. This job needs private source, so it stops here. It fetched and verified no private source. A maintainer must run this change from a branch of this repository."
fi
if [[ -z "$repository" ]]; then
  fail "The private dependency repository secret is empty. This job fetched and verified no private source."
fi
if [[ ! "$repository" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]]; then
  fail "The private dependency repository secret is not in owner/name form."
fi
# The runner masks the secret itself. Mask the bare name too, because tools print it alone.
echo "::add-mask::${repository#*/}"

# Install the toolchain that the manifest selects before the key exists.
cargo --version >/dev/null 2>&1 || fail "Cargo is not available in this job."

work=$(mktemp -d "${RUNNER_TEMP:?}/private-dependency.XXXXXX")
chmod 700 "$work"
agent_pid=
cleanup() {
  if [[ -n "$agent_pid" ]]; then
    kill "$agent_pid" 2>/dev/null || true
    # The agent is not a child of this shell. Wait until it is gone, for at most five seconds.
    for _ in $(seq 1 50); do
      kill -0 "$agent_pid" 2>/dev/null || break
      sleep 0.1
    done
  fi
  rm -rf "$work"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

# GitHub's published Ed25519 host key (https://api.github.com/meta, ssh_keys).
echo "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl" > "$work/known_hosts"

# The agent drops the key after ten minutes even when this script cannot clean up.
agent=$(ssh-agent -s -a "$work/agent.sock" -t 600)
agent_pid=$(sed -n 's/^SSH_AGENT_PID=\([0-9]*\);.*/\1/p' <<<"$agent")
[[ -n "$agent_pid" ]] || fail "The key agent did not start."
if ! printf '%s\n' "$PRIVATE_DEPENDENCY_KEY" | SSH_AUTH_SOCK="$work/agent.sock" ssh-add -q - >/dev/null 2>&1; then
  fail "The private dependency key is not a usable SSH private key."
fi
unset PRIVATE_DEPENDENCY_KEY

# These settings stay in this process. Nothing goes to the Git configuration or to GITHUB_ENV.
export GIT_SSH_COMMAND="ssh -F /dev/null -o BatchMode=yes -o IdentityAgent=$work/agent.sock -o IdentityFile=none -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$work/known_hosts -o GlobalKnownHostsFile=/dev/null -o HostKeyAlgorithms=ssh-ed25519 -o LogLevel=ERROR"
export GIT_TERMINAL_PROMPT=0
export GIT_CONFIG_COUNT=1
export GIT_CONFIG_KEY_0="url.git@github.com:$repository.insteadOf"
export GIT_CONFIG_VALUE_0="https://github.com/$repository"
export CARGO_NET_GIT_FETCH_WITH_CLI=true

# Read the private source: fetch the default branch head and require its Cargo manifest.
git init --quiet "$work/source" >/dev/null 2>&1
if ! git -C "$work/source" fetch --quiet --depth=1 "git@github.com:$repository.git" HEAD >/dev/null 2>&1; then
  fail "The key did not give read access to the private dependency. This job verified no private source."
fi
if ! git -C "$work/source" cat-file -e FETCH_HEAD:Cargo.toml >/dev/null 2>&1; then
  fail "The private dependency has no Cargo.toml at its default branch head."
fi
echo "Private source read: the default branch head has a Cargo manifest."

# Fetch every locked source now, so that later steps need no key.
if ! cargo fetch --locked --manifest-path "$manifest" >"$work/cargo.log" 2>&1; then
  echo "cargo fetch --locked failed. Output without the lines that name the private dependency:"
  grep -viF -e "$repository" -e "${repository#*/}" "$work/cargo.log" || true
  fail "Cargo could not fetch the locked sources of $manifest."
fi
echo "Locked Cargo sources fetched for $manifest."
