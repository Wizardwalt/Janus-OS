#!/usr/bin/env python3
"""Structural sanity check for Rust sources when no Rust toolchain is available.

This is NOT a compiler and does NOT prove the code builds or that tests pass. It
only verifies that delimiters balance after correctly skipping string literals,
raw string literals, character literals, lifetimes, and both comment forms. It
exists because this environment cannot run `cargo check`, and a naive regex
counter produces false positives on Rust raw strings.

Usage:
    python3 tools/audit/check_rust_structure.py [PATH ...]

With no arguments, checks every tracked .rs file under the repository.
Exits non-zero if any file has unmatched delimiters.
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

PAIRS = {"}": "{", "]": "[", ")": "("}


def unmatched(src: str) -> list[tuple[str, int]]:
    """Return (character, line_number) for every unmatched delimiter."""
    i, n = 0, len(src)
    stack: list[tuple[str, int]] = []
    line = 1

    def advance(to: int) -> None:
        nonlocal line
        line += src.count("\n", i, to)

    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            advance(j)
            i = j
            continue
        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth += 1
                    j += 2
                elif src.startswith("*/", j):
                    depth -= 1
                    j += 2
                else:
                    j += 1
            advance(j)
            i = j
            continue
        if c == "r" and i + 1 < n and src[i + 1] in '#"':
            j, hashes = i + 1, 0
            while j < n and src[j] == "#":
                hashes += 1
                j += 1
            if j < n and src[j] == '"':
                close = '"' + "#" * hashes
                k = src.find(close, j + 1)
                k = n if k < 0 else k + len(close)
                advance(k)
                i = k
                continue
        if c == '"':
            j = i + 1
            while j < n:
                if src[j] == "\\":
                    j += 2
                    continue
                if src[j] == '"':
                    j += 1
                    break
                j += 1
            advance(j)
            i = j
            continue
        if c == "'":
            # A char literal is 'x' or '\x' / '\u{...}'; anything else is a lifetime.
            if i + 1 < n and src[i + 1] == "\\":
                k = src.find("'", i + 2)
                if k > 0:
                    advance(k + 1)
                    i = k + 1
                    continue
            elif i + 2 < n and src[i + 2] == "'":
                advance(i + 3)
                i += 3
                continue
            i += 1
            continue
        if c in "{[(":
            stack.append((c, line))
        elif c in "}])":
            if stack and stack[-1][0] == PAIRS[c]:
                stack.pop()
            else:
                stack.append((c, line))
        i += 1
    return stack


def main(argv: list[str]) -> int:
    if argv:
        files = [Path(a) for a in argv]
    else:
        listing = subprocess.run(
            ["git", "ls-files", "*.rs"], capture_output=True, text=True, check=True
        ).stdout
        files = [Path(p) for p in listing.splitlines() if p]

    failures = 0
    for path in files:
        if not path.is_file():
            print(f"  MISSING {path}", file=sys.stderr)
            failures += 1
            continue
        problems = unmatched(path.read_text(encoding="utf-8", errors="replace"))
        if problems:
            failures += 1
            print(f"  FAIL {path}: {len(problems)} unmatched delimiter(s)")
            lines = path.read_text(encoding="utf-8", errors="replace").split("\n")
            for char, line_no in problems[:5]:
                text = lines[line_no - 1].strip() if 0 < line_no <= len(lines) else ""
                print(f"       {char!r} at line {line_no}: {text[:100]}")

    print(f"{len(files)} Rust files checked, {failures} with unmatched delimiters")
    if failures == 0:
        print("NOTE: structure only. This does NOT prove the code compiles or tests pass.")
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
