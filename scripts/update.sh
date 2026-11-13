#!/usr/bin/env bash
#
# Moves the tools to the newest API definitions the Rust SDK has: the SDK
# dependency to the head of origincloudhq/sdk-rust's main, API_REVISION to
# the API revision that commit was generated from, then generates the tools
# again (scripts/generate.sh), and adds the SDK crate of a service the API
# gained, whose tools call it. The regeneration workflow
# (.github/workflows/regenerate.yml) runs it daily; run it by hand the same
# way.
#
# The SDK, not the API, leads: the generated tools use the SDK's types, so
# the tools can only move to an API revision the SDK has been generated
# from.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly ROOT
readonly SDK_REPOSITORY="https://github.com/origincloudhq/sdk-rust.git"

cd "${ROOT}"

sdk="$(git ls-remote "${SDK_REPOSITORY}" refs/heads/main | cut -f1)"
[[ "${sdk}" =~ ^[0-9a-f]{40}$ ]] || { echo "no main branch in ${SDK_REPOSITORY}" >&2; exit 1; }

tmp="$(mktemp -d)"
trap 'rm -rf "${tmp}"' EXIT
git init --quiet "${tmp}/sdk"
git -C "${tmp}/sdk" fetch --quiet --depth 1 "${SDK_REPOSITORY}" "${sdk}"
revision="$(git -C "${tmp}/sdk" show FETCH_HEAD:API_REVISION | tr -d '[:space:]')"
[[ "${revision}" =~ ^[0-9a-f]{40}$ ]] || { echo "the SDK commit ${sdk:0:12} has no API_REVISION" >&2; exit 1; }

if ! grep -qF "rev = \"${sdk}\"" Cargo.toml; then
  sed -i -E "s|(git = \"https://github.com/origincloudhq/sdk-rust\", rev = \")[0-9a-f]{40}(\")|\1${sdk}\2|" Cargo.toml
  # Every crate Cargo.toml takes from the SDK.
  packages=()
  while read -r crate; do
    packages+=(-p "${crate}")
  done < <(grep -oE '^[a-z0-9-]+ = \{ git = "https://github.com/origincloudhq/sdk-rust"' Cargo.toml | cut -d' ' -f1)
  [[ "${#packages[@]}" -gt 0 ]] || { echo "Cargo.toml takes no crate from ${SDK_REPOSITORY}" >&2; exit 1; }
  cargo update --quiet "${packages[@]}"
fi
echo "${revision}" > API_REVISION
# Adds what main's Cargo.toml needs and the lock doesn't hold yet, such as
# after a lock conflict was resolved with the branch's side, so that the
# result builds with --locked; it moves no other dependency.
cargo update --quiet --workspace

# The shared rules: SKILLS_REVISION moves to the head of origincloudhq/skills
# only when its rules changed, so that the two repositories' regenerations,
# each pinning the other, don't move each other's pins for nothing.
readonly SKILLS_REPOSITORY="https://github.com/origincloudhq/skills.git"
skills="$(git ls-remote "${SKILLS_REPOSITORY}" refs/heads/main | cut -f1)"
[[ "${skills}" =~ ^[0-9a-f]{40}$ ]] || { echo "no main branch in ${SKILLS_REPOSITORY}" >&2; exit 1; }
git init --quiet "${tmp}/skills"
git -C "${tmp}/skills" fetch --quiet --depth 1 "${SKILLS_REPOSITORY}" "${skills}"
if ! git -C "${tmp}/skills" show FETCH_HEAD:shared/rules.md | cmp -s - crates/mcp/src/generated/rules.md; then
  echo "${skills}" > SKILLS_REVISION
fi

echo "origincloudhq/sdk-rust@${sdk:0:12}, origincloudhq/api@${revision:0:12}, origincloudhq/skills@$(cut -c1-12 SKILLS_REVISION)"
scripts/generate.sh

# The SDK crates the generated tools call that Cargo.toml doesn't take yet,
# a service's the API gained: each is added at the SDK commit the others are
# pinned to, to the workspace's dependencies and the library's, after the
# SDK crates there. merge-regeneration.yml leaves a pull request adding one
# for review (CONTRIBUTING.md).
readonly SDK_DEPENDENCY='^origincloud-[a-z0-9-]+ = [{] git = "https://github.com/origincloudhq/sdk-rust"'
readonly LIBRARY_DEPENDENCY='^origincloud-[a-z0-9-]+[.]workspace = true$'
# Writes LINE after the last line of FILE matching the extended regular
# expression PATTERN, the arguments in that order; fails when none does.
after() {
  PATTERN="$1" LINE="$2" awk '
    { lines[NR] = $0 } $0 ~ ENVIRON["PATTERN"] { last = NR }
    END {
      if (!last) exit 1
      for (i = 1; i <= NR; i++) { print lines[i]; if (i == last) print ENVIRON["LINE"] }
    }' "$3" > "$3.new" || { rm -f "$3.new"; echo "no line of $3 matches ${1}" >&2; exit 1; }
  mv "$3.new" "$3"
}
added=()
while read -r module; do
  crate="${module//_/-}"
  grep -qE "^${crate} = " Cargo.toml && continue
  if ! git -C "${tmp}/sdk" grep -qE "^name = \"${crate}\"$" FETCH_HEAD -- 'crates/*/Cargo.toml'; then
    echo "the generated tools call ${module}, which origincloudhq/sdk-rust@${sdk:0:12} has no crate for" >&2
    exit 1
  fi
  after "${SDK_DEPENDENCY}" "${crate} = { git = \"https://github.com/origincloudhq/sdk-rust\", rev = \"${sdk}\" }" Cargo.toml
  after "${LIBRARY_DEPENDENCY}" "${crate}.workspace = true" crates/mcp/Cargo.toml
  added+=("${crate}")
done < <(grep -ohE '::origincloud_[a-z0-9_]+::' crates/mcp/src/generated/*.rs | sed 's/^:://; s/::$//' | LC_ALL=C sort -u)
if [[ "${#added[@]}" -gt 0 ]]; then
  cargo update --quiet --workspace
  echo "added ${added[*]}: a service the API gained"
fi
