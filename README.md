# Bazel Compile Commands Extractor (Rust)

A Rust/Bazel reimplementation of [Hedron's Bazel compile commands extractor](https://github.com/hedronvision/bazel-compile-commands-extractor).

The goal is API compatibility with Hedron's public Bazel interface while moving the implementation into a Rust binary built by Bazel.

## Current interface

The repository exposes the same primary macro name as Hedron:

```starlark
load("@bazel_compile_commands_extractor_rs//:refresh_compile_commands.bzl", "refresh_compile_commands")

refresh_compile_commands(
    name = "refresh_compile_commands",
    targets = {
        "//:my_binary": "--config=clang",
    },
    exclude_headers = "external",
    exclude_external_sources = True,
)
```

Then run:

```sh
bazel run :refresh_compile_commands
```

The generated wrapper invokes the Rust `//:compile_commands_extractor` binary, which:

1. runs `bazel aquery` for C-family compile actions matching Hedron's mnemonic filter,
2. parses Bazel's `--output=jsonproto` action data,
3. writes `compile_commands.json` at `BUILD_WORKSPACE_DIRECTORY`, and
4. supports Hedron-style `targets`, `exclude_headers`, and `exclude_external_sources` options.

## Parity status

This is a base implementation intended to preserve Hedron's top-level shape first. Implemented parity includes:

- `refresh_compile_commands(...)` macro name and common arguments,
- default target behavior of `@//...`,
- string/list/dict target normalization,
- `bazel aquery` extraction with `Cpp`, `Objc`, and `Cuda` compile mnemonics,
- `compile_commands.json` entries using `arguments`, `file`, and `directory`,
- one emitted entry per header, and
- `exclude_headers` / `exclude_external_sources` filtering.

Remaining deeper Hedron parity work includes platform-specific command patching (Apple, Emscripten, NVCC, MSVC), dependency-file based header discovery, param-file spillover behavior, and the exact runfiles behavior across all Bazel versions/platforms.

## Development

Use Bazel for validation:

```sh
bazel test //:extractor_test
bazel build //:compile_commands_extractor
```
