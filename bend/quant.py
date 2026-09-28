#!/usr/bin/env python3
"""Sets the reuse marker (+) on def parameters and case-pattern variables of a Bend module.

A variable needs + when some execution path uses it more than once: uses in straight-line code
add up, while the branches of a match are alternatives, so only the busiest branch counts. A
variable used at most once on every path loses a + it does not need, which saves the reference
count on large records. Usage: python3 bend/quant.py FILE...
"""

import re
import sys

SIG = re.compile(r"^((?:@unsafe )?def [A-Za-z0-9_.]+\()(.*)(\) -> .*:)$")
CASE = re.compile(r"^(\s*)case (.*):$")


def indent(line):
    return len(line) - len(line.lstrip())


def uses(name, text):
    return len(re.findall(r"(?<![A-Za-z0-9_.+~\-])" + re.escape(name) + r"(?![A-Za-z0-9_{(])", text))


def path_count(name, lines):
    """Most uses of name along any path through a block of lines."""
    total, i = 0, 0
    while i < len(lines):
        line = lines[i]
        if CASE.match(line):
            depth = indent(line)
            best = 0
            while i < len(lines) and CASE.match(lines[i]) and indent(lines[i]) == depth:
                body, j = [], i + 1
                while j < len(lines) and (not lines[j].strip() or indent(lines[j]) > depth):
                    body.append(lines[j])
                    j += 1
                best = max(best, path_count(name, body))
                i = j
            total += best
            continue
        stripped = re.sub(r'"(?:[^"\\]|\\.)*"', '""', line)
        total += uses(name, stripped)
        i += 1
    return total


def split_params(text):
    parts, depth, current = [], 0, ""
    for ch in text:
        if ch in "<([{":
            depth += 1
        elif ch in ">)]}":
            depth -= 1
        if ch == "," and depth == 0:
            parts.append(current.strip())
            current = ""
        else:
            current += ch
    if current.strip():
        parts.append(current.strip())
    return parts


def mark(name_with_marker, count):
    bare = name_with_marker.lstrip("+")
    return ("+" if count >= 2 else "") + bare


def process(lines):
    out = list(lines)
    i = 0
    while i < len(out):
        sig = SIG.match(out[i])
        if not sig:
            i += 1
            continue
        j = i + 1
        while j < len(out) and (not out[j].strip() or out[j].startswith(" ")):
            j += 1
        body = out[i + 1 : j]
        params = []
        for param in split_params(sig.group(2)):
            m = re.match(r"^([+]?)([a-z_][A-Za-z0-9_]*)(\s*:.*)$", param)
            if m and "->" not in m.group(3) and "IO(" not in m.group(3):
                params.append(mark(m.group(1) + m.group(2), path_count(m.group(2), body)) + m.group(3))
            else:
                params.append(param)
        out[i] = sig.group(1) + ", ".join(params) + sig.group(3)
        # case patterns inside the body
        for k in range(i + 1, j):
            case = CASE.match(out[k])
            if not case:
                continue
            depth = indent(out[k])
            branch, n = [], k + 1
            while n < j and (not out[n].strip() or indent(out[n]) > depth):
                branch.append(out[n])
                n += 1

            def repl(m):
                return m.group(1) + mark(m.group(2), path_count(m.group(2).lstrip("+"), branch))

            pattern = re.sub(r"([{,]\s*|^|\s)(\+?[a-z_][A-Za-z0-9_]*)(?=\s*[,}]|$)", repl, case.group(2))
            out[k] = case.group(1) + "case " + pattern + ":"
        i = j
    return out


def main():
    for path in sys.argv[1:]:
        with open(path) as handle:
            lines = handle.read().split("\n")
        with open(path, "w") as handle:
            handle.write("\n".join(process(lines)))


if __name__ == "__main__":
    main()
