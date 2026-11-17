#!/usr/bin/env bash
# Commit and push CI output without losing races.
#
# Several jobs write to this branch, so a plain `git push` is rejected whenever
# another job committed first. Retry with a rebase; if the push still fails,
# print why and exit non-zero rather than quietly dropping the artifacts.
#
# Usage: ci-commit.sh "message" path [path...]
set -uo pipefail

msg="$1"; shift
git config user.name  "ci"
git config user.email "ci@example.invalid"

git add -A -- "$@" 2>/dev/null || git add -A

if git diff --cached --quiet; then
  echo "ci-commit: nothing to commit"
  exit 0
fi
git commit -q -m "$msg"

branch="${GITHUB_REF_NAME:-$(git rev-parse --abbrev-ref HEAD)}"
for attempt in 1 2 3 4 5 6; do
  if git push origin "HEAD:${branch}" 2>/tmp/push.err; then
    echo "ci-commit: pushed on attempt $attempt"
    exit 0
  fi
  echo "ci-commit: attempt $attempt rejected:"; sed -n '1,6p' /tmp/push.err
  # Local commits from an earlier CI push can make a rebase dirty; autostash
  # keeps the tree intact while the branch is fast-forwarded.
  git fetch -q origin "$branch" || true
  git rebase --autostash -q "origin/${branch}" 2>/dev/null || { git rebase --abort 2>/dev/null; }
  sleep $((attempt * 3))
done
echo "ci-commit: FAILED to push after 6 attempts" >&2
sed -n '1,20p' /tmp/push.err >&2
exit 1
