#!/usr/bin/env bash
# Commit and push CI output without losing races.
#
# Several jobs in this workflow write to the same branch. A plain `git push`
# fails when another job got there first, so retry with a rebase; on the last
# attempt print the error rather than silently dropping the artifacts.
set -uo pipefail

msg="$1"; shift
git config user.name  "ci"
git config user.email "ci@example.invalid"

git add -A "$@"
if git diff --cached --quiet; then
  echo "ci-commit: nothing to commit"
  exit 0
fi
git commit -q -m "$msg"

branch="${GITHUB_REF_NAME:-main}"
for attempt in 1 2 3 4 5; do
  if git push -q origin "HEAD:${branch}"; then
    echo "ci-commit: pushed on attempt $attempt"
    exit 0
  fi
  echo "ci-commit: push attempt $attempt rejected, rebasing"
  git fetch -q origin "$branch" || true
  git rebase -q "origin/${branch}" || { git rebase --abort; }
  sleep $((attempt * 3))
done
echo "ci-commit: FAILED to push after 5 attempts" >&2
exit 1
