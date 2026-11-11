#!/usr/bin/env bash
# Runs as a separate step after the fetch. It has the repository name and no key, like any later
# step. It fails when a later step could still use the key or read the private repository.
set -euo pipefail

fail() {
  echo "::error title=Private dependency::$1"
  exit 1
}

repository=${PRIVATE_DEPENDENCY_REPOSITORY:?}

[[ -z "${SSH_AUTH_SOCK:-}" ]] || fail "A key agent socket is still in the job environment."
[[ -z "${PRIVATE_DEPENDENCY_KEY:-}" ]] || fail "The key is still in the job environment."
if compgen -G "${RUNNER_TEMP:?}/private-dependency.*" >/dev/null; then
  fail "The key work directory still exists."
fi
if pgrep -x ssh-agent >/dev/null 2>&1; then
  fail "A key agent is still running."
fi
if git config --global --get-regexp '^url\..*\.insteadof$' >/dev/null 2>&1; then
  fail "A Git URL rewrite is still in the global configuration."
fi
echo "No key agent, key file or URL rewrite remains."

# Each attempt must reach GitHub and be refused. A network or host key error is not a refusal.
known_hosts=$(mktemp "$RUNNER_TEMP/known-hosts.XXXXXX")
trap 'rm -f "$known_hosts"' EXIT
echo "github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl" > "$known_hosts"
ssh_command="ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o UserKnownHostsFile=$known_hosts -o GlobalKnownHostsFile=/dev/null -o HostKeyAlgorithms=ssh-ed25519"

if output=$(GIT_TERMINAL_PROMPT=0 GIT_SSH_COMMAND="$ssh_command" git ls-remote "git@github.com:$repository.git" HEAD 2>&1); then
  fail "A step without the key read the private repository over SSH."
fi
grep -qF "Permission denied (publickey)" <<<"$output" || fail "The SSH check did not reach GitHub, so it proves nothing."
echo "SSH without the key: GitHub refused the read (Permission denied)."

if output=$(GIT_TERMINAL_PROMPT=0 git ls-remote "https://github.com/$repository" HEAD 2>&1); then
  fail "A step without the key read the private repository over HTTPS."
fi
grep -qiE "not found|could not read Username|Authentication failed|terminal prompts disabled" <<<"$output" || fail "The HTTPS check did not reach GitHub, so it proves nothing."
echo "HTTPS with the job token: GitHub refused the read."
