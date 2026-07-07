"""Entrypoint for refreshing compile_commands.json.

This file exposes the public `refresh_compile_commands` macro and delegates the
implementation to this repository's Rust binary.
"""

def refresh_compile_commands(
        name,
        targets = None,
        exclude_headers = None,
        exclude_external_sources = False,
        **kwargs):
    """Creates a `bazel run` target that writes compile_commands.json.

    Args:
      name: Name of the generated runnable target.
      targets: String, list, dict, select(), or None.
      exclude_headers: None, "all", or "external".
      exclude_external_sources: Whether to omit compile actions from external repos.
      **kwargs: Common Bazel attributes forwarded to the generated executable.
    """
    if type(targets) == "select":
        labels_to_flags = targets
    else:
        if not targets:
            targets = {"@//...": ""}
        elif type(targets) == "list":
            targets = {target: "" for target in targets}
        elif type(targets) != "dict":
            targets = {targets: ""}

        labels_to_flags = {
            target if target.startswith("/") or target.startswith("@") else "{}//{}:{}".format(native.repository_name(), native.package_name(), target.removeprefix(":")): flags
            for target, flags in targets.items()
        }

    _refresh_compile_commands_wrapper(
        name = name,
        labels_to_flags = labels_to_flags,
        exclude_headers = exclude_headers or "",
        exclude_external_sources = exclude_external_sources,
        **kwargs
    )

def _refresh_compile_commands_wrapper_impl(ctx):
    script = ctx.outputs.executable
    arguments = []
    for target, flags in ctx.attr.labels_to_flags.items():
        arguments.extend(["--target", "{}={}".format(target, flags)])

    if ctx.attr.exclude_headers:
        arguments.extend(["--exclude_headers", ctx.attr.exclude_headers])

    if ctx.attr.exclude_external_sources:
        arguments.append("--exclude_external_sources")

    ctx.actions.write(
        output = script,
        is_executable = True,
        content = "\n".join([
            "#!/usr/bin/env bash",
            "set -euo pipefail",
            "runfiles_dir=\"${RUNFILES_DIR:-${0}.runfiles}\"",
            "extractor=\"${{runfiles_dir}}/{}\"".format(ctx.executable._extractor.short_path),
            "if [[ ! -x \"${extractor}\" ]]; then",
            "  extractor=\"${{runfiles_dir}}/_main/{}\"".format(ctx.executable._extractor.short_path),
            "fi",
            "if [[ ! -x \"${extractor}\" ]]; then",
            "  extractor=\"$(find \"${runfiles_dir}\" -type f -name compile_commands_extractor -perm -111 2>/dev/null | sort | head -n 1)\"",
            "fi",
            "if [[ ! -x \"${extractor}\" ]]; then",
            "  echo 'Could not locate compile_commands_extractor in Bazel runfiles.' >&2",
            "  exit 1",
            "fi",
            "exec \"${{extractor}}\" {} -- \"$@\"".format(" ".join([repr(argument) for argument in arguments])),
            "",
        ]),
    )
    return DefaultInfo(
        executable = script,
        files = depset([script]),
        runfiles = ctx.runfiles(files = [ctx.executable._extractor]),
    )

_refresh_compile_commands_wrapper = rule(
    executable = True,
    attrs = {
        "labels_to_flags": attr.string_dict(mandatory = True),
        "exclude_external_sources": attr.bool(default = False),
        "exclude_headers": attr.string(values = ["all", "external", ""]),
        "_extractor": attr.label(
            executable = True,
            cfg = "exec",
            default = Label("//:compile_commands_extractor"),
        ),
    },
    implementation = _refresh_compile_commands_wrapper_impl,
)
