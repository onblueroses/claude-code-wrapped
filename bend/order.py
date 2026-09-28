#!/usr/bin/env python3
"""Reorders a Bend module so every def follows the defs it calls.

Bend checks a def only against the defs above it (an @unsafe def may look below). This keeps
each block's leading comments with it and otherwise preserves the written order, moving a def
only as far down as its dependencies require. Usage: python3 bend/order.py FILE...
"""

import re
import sys

HEAD = re.compile(r"^(?:@unsafe\s+)?(?:def|law|type)\s+([A-Za-z0-9_.]+)")


def blocks(text):
    """Split into (name, text) top-level blocks; the preamble has name None."""
    lines = text.split("\n")
    out, current, name, pending = [], [], None, []
    for line in lines:
        match = HEAD.match(line)
        if match:
            if current:
                out.append((name, "\n".join(current).rstrip("\n")))
            current, name = pending + [line], match.group(1)
            pending = []
        elif line.startswith("#") and current and (not current[-1].strip() or current[-1].startswith("#")) and name is not None:
            pending.append(line)
        elif not line.strip() and pending:
            pending.append(line)
        else:
            if pending:
                current.extend(pending)
                pending = []
            current.append(line)
    if current or pending:
        out.append((name, "\n".join(current + pending).rstrip("\n")))
    return out


def reorder(text):
    parts = blocks(text)
    preamble = [p for p in parts if p[0] is None]
    body = [p for p in parts if p[0] is not None]
    names = {name for name, _ in body}
    deps = {}
    for name, chunk in body:
        body_text = chunk.split("\n", 1)[1] if "\n" in chunk else ""
        header = chunk.split("\n", 1)[0]
        unsafe = header.startswith("@unsafe") or any(l.startswith("@unsafe") for l in chunk.split("\n") if HEAD.match(l.lstrip()))
        refs = set(re.findall(r"(?<![A-Za-z0-9_.])([A-Z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_]+)+)\s*\(", body_text))
        deps[name] = set() if unsafe else {r for r in refs if r in names and r != name}
    placed, order = set(), []
    remaining = list(body)
    while remaining:
        for i, (name, chunk) in enumerate(remaining):
            if deps[name] <= placed:
                order.append((name, chunk))
                placed.add(name)
                del remaining[i]
                break
        else:
            raise SystemExit("cycle among safe defs: " + ", ".join(n for n, _ in remaining))
    return "\n\n".join(c for _, c in preamble + order) + "\n"


def main():
    for path in sys.argv[1:]:
        with open(path) as handle:
            text = handle.read()
        ordered = reorder(text)  # raises on a cycle before the file is touched
        with open(path, "w") as handle:
            handle.write(ordered)


if __name__ == "__main__":
    main()
