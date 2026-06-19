# Compile commands comparison example

This example compares this Rust extractor against the reference extractor on the same small C++ Bazel workspace.

## Workspaces

- `local/` uses this repository via Bzlmod:

  ```starlark
  bazel_dep(name = "bazel_compile_commands", dev_dependency = True)
  local_path_override(
      module_name = "bazel_compile_commands",
      path = "../../..",
  )
  ```

- `reference/` uses the requested reference revision:

  ```starlark
  bazel_dep(name = "hedron_compile_commands", dev_dependency = True)
  git_override(
      module_name = "hedron_compile_commands",
      commit = "7fe1eab26d2b8eeb5e1c6a2f38bddb001e3f9696",
      remote = "https://github.com/hedronvision/bazel-compile-commands-extractor.git",
      # Note: this is a side branch compatible with Bazel 9 with the changes to rules_python.
  )
  ```

Both workspaces define the same `:hello` C++ binary and `:refresh_compile_commands` target.

## Run

From this directory:

```sh
./compare.sh
```

The script writes:

- `local/compile_commands.json`
- `reference/compile_commands.json`
- `compile_commands.raw.diff`
- `local.compile_commands.normalized.json`
- `reference.compile_commands.normalized.json`
- `compile_commands.normalized.diff`

The normalized diff sorts entries and replaces each workspace root with `${WORKSPACE}` so differences are easier to review.
