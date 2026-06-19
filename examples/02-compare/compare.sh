#!/usr/bin/env bash
set -euo pipefail

example_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
local_workspace="${example_root}/local"
reference_workspace="${example_root}/reference"

(
  cd "${local_workspace}"
  rm -f compile_commands.json
  bazel run :refresh_compile_commands
)

(
  cd "${reference_workspace}"
  rm -f compile_commands.json
  bazel run :refresh_compile_commands
)

python3 "${example_root}/normalize_compile_commands.py" "${local_workspace}" "${local_workspace}/compile_commands.json" > "${example_root}/local.compile_commands.normalized.json"
python3 "${example_root}/normalize_compile_commands.py" "${reference_workspace}" "${reference_workspace}/compile_commands.json" > "${example_root}/reference.compile_commands.normalized.json"

set +e
diff -u "${local_workspace}/compile_commands.json" "${reference_workspace}/compile_commands.json" > "${example_root}/compile_commands.raw.diff"
raw_status="$?"
diff -u "${example_root}/local.compile_commands.normalized.json" "${example_root}/reference.compile_commands.normalized.json" > "${example_root}/compile_commands.normalized.diff"
normalized_status="$?"
set -e

printf 'Raw diff:        %s\n' "${example_root}/compile_commands.raw.diff"
printf 'Normalized diff: %s\n' "${example_root}/compile_commands.normalized.diff"

if [[ "${raw_status}" -eq 0 ]]; then
  printf 'Raw compile_commands.json files match.\n'
else
  printf 'Raw compile_commands.json files differ.\n'
fi

if [[ "${normalized_status}" -eq 0 ]]; then
  printf 'Normalized compile_commands.json files match.\n'
else
  printf 'Normalized compile_commands.json files differ.\n'
fi
