#!/usr/bin/env bash
# Fetch TypeScript's test suite into _submodules/typescript.
#
# The TypeScript repo is large, so this does a shallow, sparse, blob-filtered
# checkout of only what the conformance runner needs: the test inputs and the
# TS 7 reference baselines (as .errors.txt, plus .js/.symbols as evidence of
# which test configurations tsc ran). That is ~90 MB instead of several GB.
#
# The commit is whatever the superproject pins for the submodule; bump it with:
#   git -C _submodules/typescript fetch --depth 1 origin tag <tag>
#   git -C _submodules/typescript checkout <tag>
#   git add _submodules/typescript
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
SUB=_submodules/typescript

SPARSE_PATTERNS=(
  '/tests/cases/compiler/'
  '/tests/cases/conformance/'
  '/tsc/testdata/baselines/reference/submodule/*/*.errors.txt'
  '/tsc/testdata/baselines/reference/submodule/*/*.js'
  '/tsc/testdata/baselines/reference/submodule/*/*.symbols'
)

url=$(git config -f .gitmodules "submodule.$SUB.url")
sha=$(git ls-files --stage -- "$SUB" | awk '{print $2}')
if [[ -z "$sha" ]]; then
  echo "error: $SUB is not a registered submodule" >&2
  exit 1
fi

git submodule init -- "$SUB" >/dev/null

if [[ ! -e "$SUB/.git" ]]; then
  echo "Initializing $SUB..."
  mkdir -p "$SUB"
  git -C "$SUB" init --quiet
  git -C "$SUB" remote add origin "$url"
fi

git -C "$SUB" sparse-checkout set --no-cone "${SPARSE_PATTERNS[@]}"

if [[ "$(git -C "$SUB" rev-parse --verify --quiet HEAD || true)" != "$sha" ]]; then
  echo "Fetching TypeScript @ ${sha:0:12} (shallow, sparse)..."
  git -C "$SUB" fetch --quiet --depth 1 --filter=blob:none origin "$sha"
  git -C "$SUB" -c advice.detachedHead=false checkout --quiet "$sha"
fi

# Store the submodule's git dir under .git/modules like `git submodule update` would.
git submodule absorbgitdirs -- "$SUB" >/dev/null 2>&1 || true

echo "TypeScript test suite ready at $SUB ($(git -C "$SUB" rev-parse --short HEAD))"
