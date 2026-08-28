#!/usr/bin/env python3
"""Verify that every source citation in specs/ points at a symbol that still exists.

Specs cite source as `<path>:<symbol>`, e.g. `src/errors.rs:classify_status`. A line
number is not a stable address: any edit above it silently invalidates the citation, so
the docs rot on every refactor with nothing to catch it. A symbol name breaks loudly the
moment the symbol is renamed or deleted, which is the case worth catching.

Usage:
    scripts/check_spec_citations.py [--specs DIR] [--root DIR]

Exits non-zero and lists every bad citation on failure.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# A citation inside backticks: a path ending in .rs, a colon, then a symbol.
# The symbol is a Rust identifier, optionally `Type::method` to name an item in an impl.
CITATION = re.compile(r"`(?P<path>[A-Za-z0-9_./-]+\.rs):(?P<symbol>[A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*)`")

# Anything that still cites a bare line number, which is what this check exists to prevent.
LINE_CITATION = re.compile(r"`(?P<path>[A-Za-z0-9_./-]+\.rs):(?P<lines>[0-9]+(?:-[0-9]+)?)`")


def definition_patterns(symbol: str) -> list[re.Pattern[str]]:
    """Patterns that count as *defining* `symbol` in a Rust source file.

    Deliberately broad. The goal is to catch a citation whose symbol was renamed or
    removed, not to model Rust's grammar. A false positive (matching something that is
    not really a definition) costs nothing; a false negative would fail a valid citation
    and train people to ignore the check.
    """
    s = re.escape(symbol)
    return [
        re.compile(rf"\bfn\s+{s}\b"),
        re.compile(rf"\b(?:struct|enum|trait|union|type|const|static|mod)\s+{s}\b"),
        re.compile(rf"\bmacro_rules!\s*{s}\b"),
        re.compile(rf"\bimpl\b[^\n]*\bfor\s+{s}\b"),
        re.compile(rf"\bimpl\b(?:<[^>]*>)?\s+{s}\b"),
        # Enum variants and struct fields: an indented identifier introducing a body,
        # a tuple, a type, or a bare unit variant.
        re.compile(rf"^\s+{s}\s*[({{:,]", re.MULTILINE),
        re.compile(rf"^\s+{s}\s*$", re.MULTILINE),
        # A cited macro-generated setter exists only inside the macro that emits it.
        re.compile(rf"\b{s}\s*\("),
    ]


def resolve(path: str, root: Path) -> Path | None:
    """Resolve a cited path against the repo root, then against src/."""
    for candidate in (root / path, root / "src" / path):
        if candidate.is_file():
            return candidate
    return None


def self_test() -> int:
    """Prove the check fails on the cases it exists to catch.

    A checker that only ever passes is indistinguishable from one that does nothing, so
    the failure paths are exercised here rather than assumed. Run by `make check/specs`.
    """
    import tempfile

    cases: list[tuple[str, str, bool]] = [
        # (description, spec body, expected_ok)
        ("a symbol that exists", "See `src/lib.rs:kept_fn`.", True),
        ("a renamed symbol", "See `src/lib.rs:renamed_away`.", False),
        ("a line number", "See `src/lib.rs:42`.", False),
        ("a missing file", "See `src/nope.rs:kept_fn`.", False),
        ("a struct", "See `src/lib.rs:KeptStruct`.", True),
        ("an enum variant", "See `src/lib.rs:KeptVariant`.", True),
        ("a const", "See `src/lib.rs:KEPT_CONST`.", True),
        ("a macro", "See `src/lib.rs:kept_macro`.", True),
        ("a Type::method form", "See `src/lib.rs:KeptStruct::kept_fn`.", True),
        ("prose with no citation", "Just `kept_fn` in prose.", True),
    ]

    source = (
        "pub fn kept_fn() {}\n"
        "pub struct KeptStruct;\n"
        "pub enum E {\n    KeptVariant,\n}\n"
        "pub const KEPT_CONST: u8 = 1;\n"
        "macro_rules! kept_macro { () => {}; }\n"
    )

    failures = []
    for description, body, expected_ok in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "src").mkdir()
            (root / "src" / "lib.rs").write_text(source, encoding="utf-8")
            (root / "specs").mkdir()
            (root / "specs" / "t.md").write_text(body, encoding="utf-8")

            argv = sys.argv
            stderr, stdout = sys.stderr, sys.stdout
            try:
                sys.argv = ["check", "--root", str(root)]
                with open("/dev/null", "w") as devnull:
                    sys.stderr = sys.stdout = devnull
                    actual_ok = main() == 0
            finally:
                sys.argv, sys.stderr, sys.stdout = argv, stderr, stdout

            if actual_ok != expected_ok:
                verb = "accepted" if actual_ok else "rejected"
                failures.append(f"  {description}: {verb}, expected the opposite")

    if failures:
        print("self-test failed:", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        return 1

    print(f"self-test ok ({len(cases)} cases)")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--specs", default="specs", help="directory of spec markdown")
    parser.add_argument("--root", default=".", help="repo root")
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="check that the checker rejects renamed symbols and line numbers",
    )
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    root = Path(args.root).resolve()
    specs = (root / args.specs).resolve()
    if not specs.is_dir():
        print(f"error: no spec directory at {specs}", file=sys.stderr)
        return 2

    sources: dict[Path, str] = {}
    failures: list[str] = []
    checked = 0

    for doc in sorted(specs.rglob("*.md")):
        text = doc.read_text(encoding="utf-8")
        rel = doc.relative_to(root)

        for lineno, line in enumerate(text.splitlines(), start=1):
            for match in LINE_CITATION.finditer(line):
                failures.append(
                    f"{rel}:{lineno}: cites a line number "
                    f"`{match.group('path')}:{match.group('lines')}`; "
                    f"cite the enclosing symbol instead"
                )

            for match in CITATION.finditer(line):
                path, symbol = match.group("path"), match.group("symbol")
                # Already reported above as a line citation; do not double-report it
                # as a missing symbol named "189".
                if symbol.isdigit():
                    continue
                checked += 1

                resolved = resolve(path, root)
                if resolved is None:
                    failures.append(f"{rel}:{lineno}: no such file `{path}`")
                    continue

                if resolved not in sources:
                    sources[resolved] = resolved.read_text(encoding="utf-8")
                body = sources[resolved]

                # For `Type::method`, the method is what has to exist; the type is context.
                leaf = symbol.split("::")[-1]
                if not any(p.search(body) for p in definition_patterns(leaf)):
                    failures.append(
                        f"{rel}:{lineno}: `{path}` defines no `{leaf}` "
                        f"(cited as `{path}:{symbol}`)"
                    )

    if failures:
        print(f"{len(failures)} bad spec citation(s):", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1

    print(f"spec citations ok ({checked} checked across {len(sources)} source files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
