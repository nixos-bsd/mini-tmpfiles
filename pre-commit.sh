#!/usr/bin/env bash
# https://stackoverflow.com/questions/26886363/git-how-to-re-stage-the-staged-files-in-a-pre-commit-hook/39521255#39521255
STASH_NAME="pre-commit-$(date +%s)"
git stash save --quiet --keep-index --include-untracked "$STASH_NAME"

./tests.sh

RESULT=$?
if [ $RESULT -ne 0 ]; then
	git stash save -q "original index"
	git stash apply -q --index "stash@{1}"
	git stash drop -q; git stash drop -q
fi
[ $RESULT -ne 0 ] && exit 1

git add -u
