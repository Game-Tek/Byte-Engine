#!/usr/bin/env python3
"""Rejects `static` items that don't say why they have no single owner.

AGENTS.md bans `static` items by default (API Design). A `static` that survives carries an `// ALLOW:` comment
right above it, or above the attributes on it, that says why no single owner works, the way an `unsafe` block
carries a `// SAFETY:` comment:

    // ALLOW: A global allocator must be a `static`: Rust installs it by its fixed address.
    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

Clippy has no lint for `static` items, so this script checks them. Run it from the repository root:

    python3 scripts/check-statics.py

It prints each unmarked `static` and exits with 1 when there is one.
"""

import pathlib
import re
import sys

# A `static` item: optional visibility, then `static` or `static mut`, its name, and the colon before its type.
# `&'static` never matches, because the keyword has to start the line, and shader code in string literals, such as
# HLSL's `static const uint tag`, has no colon after the name.
STATIC_ITEM = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(unsafe\s+)?static\s+(mut\s+)?[A-Za-z_][A-Za-z0-9_]*\s*:")
# Lines that may sit between an item and its `// ALLOW:` comment: attributes and other comment lines.
ATTRIBUTE = re.compile(r"^\s*#!?\[")
COMMENT = re.compile(r"^\s*//")
ALLOW = re.compile(r"^\s*// ALLOW: \S")


def allowed(lines: list[str], index: int) -> bool:
    """Returns whether the comments and attributes right above line `index` hold an `// ALLOW:` comment."""
    above = index - 1
    while above >= 0 and (ATTRIBUTE.match(lines[above]) or COMMENT.match(lines[above])):
        if ALLOW.match(lines[above]):
            return True
        above -= 1
    return False


def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    unmarked = []
    for path in sorted((root / "crates").rglob("*.rs")):
        if "target" in path.relative_to(root).parts:
            continue
        lines = path.read_text(encoding="utf-8").splitlines()
        for index, line in enumerate(lines):
            if STATIC_ITEM.match(line) and not allowed(lines, index):
                unmarked.append(f"{path.relative_to(root)}:{index + 1}: {line.strip()}")
    for item in unmarked:
        print(item)
    if unmarked:
        print(
            f"\nFound {len(unmarked)} `static` items without an `// ALLOW:` comment. The most likely cause is state "
            "that has no owner yet. Keep it in its owner, or use a `const` for a value known at compile time; if no "
            "single owner can work, say why in an `// ALLOW:` comment right above the item (see AGENTS.md, API Design).",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
