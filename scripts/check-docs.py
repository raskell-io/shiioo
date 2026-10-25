#!/usr/bin/env python3
"""Checks every relative link in the repository's Markdown files.

Usage: python3 scripts/check-docs.py   (exit code 1 if any link is broken)

Covers README.md, docs/**, crates/**/README.md and crates/**/docs/**. External
links (http, https, mailto) are not fetched. A link to a directory counts as
resolved if the directory exists.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LINK = re.compile(r"\]\(([^)\s]+?)(?:#[^)]*)?\)")


def markdown_files():
    yield ROOT / "README.md"
    yield from (ROOT / "docs").rglob("*.md")
    for crate in (ROOT / "crates").iterdir():
        readme = crate / "README.md"
        if readme.exists():
            yield readme
        docs = crate / "docs"
        if docs.is_dir():
            yield from docs.rglob("*.md")


def main() -> int:
    broken = []
    for md in markdown_files():
        text = md.read_text(encoding="utf-8")
        for match in LINK.finditer(text):
            target = match.group(1)
            if target.startswith(("http://", "https://", "mailto:")) or target.startswith("#"):
                continue
            if not (md.parent / target).exists():
                line = text.count("\n", 0, match.start()) + 1
                broken.append(f"{md.relative_to(ROOT)}:{line}: {target}")
    for item in broken:
        print(item)
    print(f"{len(broken)} broken link(s)")
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main())
