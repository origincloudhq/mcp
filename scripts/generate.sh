#!/usr/bin/env bash
#
# Generates the MCP tools of every service of the API (`pvs_create_volume`,
# `location_list_zones`, ...) from the API definitions in origincloudhq/api
# at the commit API_REVISION names, with this repository's generator
# (generator/), built at the versions Cargo.lock pins, into
# crates/mcp/src/generated/.
#
#   scripts/generate.sh           # generate them again
#   scripts/generate.sh --check   # check the checked-in code is current
#
# The tools use the Rust SDK's request and response types, so the SDK
# commit Cargo.lock pins must have been generated from the same API
# revision: both modes check that its API_REVISION is this one.
#
# The server's instructions are the rules every Origin Cloud skill shares,
# origincloudhq/skills' shared/rules.md at the commit SKILLS_REVISION names,
# copied to crates/mcp/src/generated/rules.md: that repository holds them,
# and this one never edits them.
#
# Needs cargo, git and network access: the API's dependencies are
# downloaded from the Buf Schema Registry at the commits its buf.lock pins,
# and checked against the digests it pins.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
readonly API_REPOSITORY="https://github.com/origincloudhq/api.git"
readonly SDK_REPOSITORY="https://github.com/origincloudhq/sdk-rust.git"
readonly SKILLS_REPOSITORY="https://github.com/origincloudhq/skills.git"
readonly GENERATED=("crates/mcp/src/generated")

case "${1:-}" in
  "") mode="write" ;;
  --check) mode="check" ;;
  *) echo "usage: $0 [--check]" >&2; exit 2 ;;
esac

revision="$(tr -d '[:space:]' < "${ROOT}/API_REVISION")"
[[ "${revision}" =~ ^[0-9a-f]{40}$ ]] || { echo "API_REVISION must hold a full commit hash" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT

# The SDK commit Cargo.lock pins, and the API revision it was generated from.
sdk="$(grep -m1 -o 'git+https://github.com/origincloudhq/sdk-rust?rev=[0-9a-f]*#[0-9a-f]\{40\}' "${ROOT}/Cargo.lock" | sed 's/.*#//')"
[[ "${sdk}" =~ ^[0-9a-f]{40}$ ]] || { echo "Cargo.lock pins no commit of ${SDK_REPOSITORY}" >&2; exit 1; }
git init --quiet "${tmp}/sdk"
git -C "${tmp}/sdk" fetch --quiet --depth 1 "${SDK_REPOSITORY}" "${sdk}"
sdk_revision="$(git -C "${tmp}/sdk" show FETCH_HEAD:API_REVISION | tr -d '[:space:]')"
if [[ "${sdk_revision}" != "${revision}" ]]; then
  echo "FAILED: the SDK commit ${sdk:0:12} was generated from origincloudhq/api@${sdk_revision:0:12}, not API_REVISION's ${revision:0:12}; run scripts/update.sh, or pin the SDK commit generated from it"
  exit 1
fi

git init --quiet "${tmp}/api"
git -C "${tmp}/api" fetch --quiet --depth 1 "${API_REPOSITORY}" "${revision}"
git -C "${tmp}/api" checkout --quiet FETCH_HEAD
(cd "${ROOT}" && cargo run --quiet --locked -p origincloud-mcp-generator -- "${tmp}/api" "${tmp}/gen")

# The shared rules, the server's instructions.
skills="$(tr -d '[:space:]' < "${ROOT}/SKILLS_REVISION")"
[[ "${skills}" =~ ^[0-9a-f]{40}$ ]] || { echo "SKILLS_REVISION must hold a full commit hash" >&2; exit 1; }
git init --quiet "${tmp}/skills"
git -C "${tmp}/skills" fetch --quiet --depth 1 "${SKILLS_REPOSITORY}" "${skills}"
git -C "${tmp}/skills" show FETCH_HEAD:shared/rules.md > "${tmp}/gen/crates/mcp/src/generated/rules.md"

# The generated files, relative to the repository.
generated() {
  local dir
  for dir in "${GENERATED[@]}"; do
    [[ -d "$1/${dir}" ]] || continue
    (cd "$1" && find "${dir}" -type f)
  done | LC_ALL=C sort
}

if [[ "${mode}" == write ]]; then
  generated "${ROOT}" | (cd "${ROOT}" && xargs -r rm -f)
  generated "${tmp}/gen" | (cd "${tmp}/gen" && xargs -r cp --parents -t "${ROOT}")
  echo "wrote $(generated "${ROOT}" | wc -l) files from origincloudhq/api@${revision:0:12}"
  exit 0
fi

failed=0
if ! diff <(generated "${ROOT}") <(generated "${tmp}/gen") >/dev/null; then
  failed=1
else
  while read -r file; do
    cmp -s "${ROOT}/${file}" "${tmp}/gen/${file}" || failed=1
  done < <(generated "${ROOT}")
fi
if [[ "${failed}" -eq 1 ]]; then
  echo "FAILED: the generated code isn't what origincloudhq/api@${revision:0:12} generates; run $0"
  exit 1
fi
echo "ok - the generated code is current (origincloudhq/api@${revision:0:12})"
