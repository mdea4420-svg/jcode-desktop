#!/usr/bin/env bash
# Claude Code PreToolUse hook: the tool call arrives as JSON on stdin.
# Exit 2 blocks the call; stderr is shown to the model as the reason.
input=$(cat)
deny() {
  echo "safety-guard: $1. Ask the user before doing this." >&2
  exit 2
}
case "$input" in
  *'rm -rf /"'* | *'rm -rf / '* | *'rm -rf ~"'* | *'rm -rf ~ '* | *'rm -rf ~/"'* | *'rm -rf *'*)
    deny "recursive delete of the root, home or a wildcard" ;;
  *'git push --force'* | *'git push -f'*) deny "force push" ;;
  *'git reset --hard'*) deny "hard reset can discard uncommitted work" ;;
  *'git clean -fd'*) deny "git clean deletes untracked files" ;;
esac
exit 0
