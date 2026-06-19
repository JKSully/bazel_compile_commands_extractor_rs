#!/usr/bin/env python3
"""Normalize compile_commands.json enough to make local/reference diffs readable."""

import json
import pathlib
import sys


def normalize_path(value: str, workspace: pathlib.Path) -> str:
    value = value.replace(str(workspace), "${WORKSPACE}")
    value = value.replace(str(workspace.parent), "${WORKSPACE_PARENT}")
    return value


def main() -> int:
    if len(sys.argv) != 3:
        print(
            "usage: normalize_compile_commands.py WORKSPACE COMPILE_COMMANDS_JSON",
            file=sys.stderr,
        )
        return 2

    workspace = pathlib.Path(sys.argv[1]).resolve()
    input_path = pathlib.Path(sys.argv[2]).resolve()
    entries = json.loads(input_path.read_text())

    normalized = []
    for entry in entries:
        normalized.append(
            {
                "file": normalize_path(entry["file"], workspace),
                "arguments": [
                    normalize_path(argument, workspace)
                    for argument in entry.get("arguments", [])
                ],
                "directory": "${WORKSPACE}",
            }
        )

    normalized.sort(key=lambda entry: (entry["file"], entry["arguments"]))
    json.dump(normalized, sys.stdout, indent=2)
    print()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
