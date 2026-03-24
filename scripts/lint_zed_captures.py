#!/usr/bin/env python3

"""Validate highlight captures against Zed's syntax capture names."""

from __future__ import annotations

import re
import sys
from pathlib import Path


CAPTURE_PATTERN = re.compile(r"@([A-Za-z0-9_.-]+)")
ALLOWED_ZED_CAPTURES = {
    "attribute",
    "boolean",
    "comment",
    "comment.doc",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "emphasis",
    "emphasis.strong",
    "enum",
    "function",
    "function.builtin",
    "hint",
    "keyword",
    "label",
    "link_text",
    "link_uri",
    "namespace",
    "number",
    "operator",
    "predictive",
    "preproc",
    "primary",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.list_marker",
    "punctuation.markup",
    "punctuation.special",
    "selector",
    "selector.pseudo",
    "string",
    "string.escape",
    "string.regex",
    "string.special",
    "string.special.symbol",
    "tag",
    "text",
    "text.literal",
    "title",
    "type",
    "variable",
    "variable.special",
    "variable.parameter",
    "variant",
}


def is_allowed(capture: str) -> bool:
    if capture.startswith("_"):
        return True

    parts = capture.split(".")
    for i in range(len(parts), 0, -1):
        prefix = ".".join(parts[:i])
        if prefix in ALLOWED_ZED_CAPTURES:
            return True
    return False


def collect_highlight_files(root: Path) -> list[Path]:
    return sorted(root.glob("languages/*/highlights.scm"))


def main() -> int:
    root = Path.cwd()
    highlight_files = collect_highlight_files(root)
    if not highlight_files:
        print("No highlights.scm files found under languages/*/")
        return 1

    failures: list[tuple[Path, int, str]] = []
    for highlight_file in highlight_files:
        text = highlight_file.read_text(encoding="utf-8")
        for line_number, line in enumerate(text.splitlines(), start=1):
            for match in CAPTURE_PATTERN.finditer(line):
                capture = match.group(1)
                if not is_allowed(capture):
                    failures.append((highlight_file, line_number, capture))

    if failures:
        print("Unsupported highlight captures detected:")
        for path, line_number, capture in failures:
            relative_path = path.relative_to(root)
            print(f"- {relative_path}:{line_number} uses @{capture}")
        print("\nUse Zed capture names (or subscopes of them) for highlights.")
        return 1

    print("All highlight captures are compatible with Zed capture names.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
