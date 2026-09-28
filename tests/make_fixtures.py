#!/usr/bin/env python3
"""Writes the edge-case transcript corpus under tests/fixtures/edge/.

Every case is a transcript line (or a raw byte string) chosen to exercise one rule of
docs/semantics.md. The engine and the oracle must agree on the whole corpus; run
tests/compare.py against it with HOME=/home/u.
"""

import json
import os
from pathlib import Path

OUT = Path(__file__).resolve().parent / "fixtures" / "edge"
HOME = "/home/u"


def line(obj):
    return json.dumps(obj, ensure_ascii=False).encode() + b"\n"


def usage(inp=3, out=10, read=100, write=50, w1h=None, w5m=None, ws=0, wf=0, speed="standard", geo="not_available", extra=None):
    u = {
        "input_tokens": inp,
        "cache_creation_input_tokens": write,
        "cache_read_input_tokens": read,
        "output_tokens": out,
        "server_tool_use": {"web_search_requests": ws, "web_fetch_requests": wf},
        "service_tier": "standard",
        "cache_creation": {
            "ephemeral_1h_input_tokens": write if w1h is None else w1h,
            "ephemeral_5m_input_tokens": 0 if w5m is None else w5m,
        },
        "inference_geo": geo,
        "speed": speed,
    }
    if extra:
        u.update(extra)
    return u


def assistant(mid, ts, model="claude-opus-5", content=None, u=None, session="s-main", cwd=HOME + "/proj", side=False, **extra):
    rec = {
        "parentUuid": None,
        "isSidechain": side,
        "cwd": cwd,
        "sessionId": session,
        "message": {
            "model": model,
            "id": mid,
            "type": "message",
            "role": "assistant",
            "content": content if content is not None else [{"type": "text", "text": "ok"}],
            "usage": u if u is not None else usage(),
        },
        "requestId": "req_" + mid,
        "type": "assistant",
        "uuid": "u-" + mid + ts,
        "timestamp": ts,
    }
    rec.update(extra)
    return rec


def user(uuid, ts, content, meta=False, side=False, session="s-main", cwd=HOME + "/proj"):
    return {
        "parentUuid": None,
        "isSidechain": side,
        "cwd": cwd,
        "sessionId": session,
        "type": "user",
        "message": {"role": "user", "content": content},
        "uuid": uuid,
        "timestamp": ts,
        **({"isMeta": True} if meta else {}),
    }


def tool(tid, name, inp=None):
    return {"type": "tool_use", "id": tid, "name": name, "input": inp or {"command": "ls"}}


def write(name, chunks):
    path = OUT / name
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as handle:
        for chunk in chunks:
            handle.write(chunk if isinstance(chunk, bytes) else line(chunk))


def main():
    if OUT.exists():
        for p in sorted(OUT.rglob("*"), reverse=True):
            p.unlink() if p.is_file() else p.rmdir()
    T = "2026-05-01T10:00:00.000Z"

    # Streaming: one response, three lines with growing output; the copy in a resumed session
    # repeats it with a smaller count and a later session id.
    write("proj-a/stream.jsonl", [
        assistant("msg_stream", T, content=[{"type": "thinking", "thinking": "", "signature": "x" * 300}], u=usage(out=5)),
        assistant("msg_stream", T, content=[{"type": "text", "text": "hi"}], u=usage(out=40)),
        assistant("msg_stream", T, content=[tool("toolu_1", "Bash")], u=usage(out=90)),
    ])
    write("proj-a/resumed.jsonl", [
        assistant("msg_stream", T, content=[tool("toolu_1", "Bash")], u=usage(out=60), session="s-zz-resumed"),
        assistant("msg_stream", "2026-05-01T09:59:59.000Z", u=usage(out=10), session="s-zz-resumed"),
    ])

    # Duplicate keys: the last occurrence wins, including whole objects.
    dup = (
        b'{"type":"assistant","timestamp":"2026-05-02T10:00:00Z","sessionId":"s-dup","cwd":"/home/u/dup",'
        b'"message":{"id":"msg_dup_old","usage":{"input_tokens":999}},'
        b'"message":{"id":"msg_dup","model":"claude-opus-5","model":"claude-sonnet-5",'
        b'"usage":{"input_tokens":1,"input_tokens":7,"output_tokens":3,"cache_creation":{"ephemeral_1h_input_tokens":5},'
        b'"cache_creation":{"ephemeral_5m_input_tokens":2},"cache_creation_input_tokens":2}}}\n'
    )
    dup_usage_array = b'{"type":"assistant","timestamp":"2026-05-02T10:00:00Z","message":{"id":"msg_dup_arr","usage":{"input_tokens":1},"usage":[1,2]}}\n'
    write("proj-b/dupkeys.jsonl", [dup, dup_usage_array])

    # Malformed lines, each counted once and otherwise ignored.
    good = line(assistant("msg_after_bad", "2026-05-03T10:00:00Z"))
    bad = [
        b'{"a":1,}\n', b'{"a":NaN}\n', b'{"a":Infinity}\n', b'{"a":-Infinity}\n', b'{"a":"unterminated}\n',
        b'{"a":"raw\ttab"}\n', b'{"a":"bad \\x escape"}\n', b'{"a":"\\u12"}\n', b'{"a":01}\n', b'{"a":-}\n',
        b'{"a":1.}\n', b'{"a":1e}\n', b'[1,2,3]\n', b'\n', b'   \n', b'\xef\xbb\xbf{"a":1}\n', b'{"a":"\xff"}\n',
        b'{"a":"\xc0\xaf"}\n', b'{"a":"\xed\xa0\x80"}\n', b'{"a":tru}\n', b'{"a":1}}\n', b'{"a":1} x\n', b'"just a string"\n',
        b'{"a":[1,2}\n', b'{"a" 1}\n', b'{a:1}\n', b"{'a':1}\n", b'{"a":1,"b"}\n', b'nul\n',
    ]
    write("proj-b/malformed.jsonl", bad + [good, b'{"type":"assistant","timestamp":"2026-05-03T10:00:00Z","message":{"id":"no_newline_tail"'])

    # Numbers: only non-negative integers count; floats, negatives and exponents read as 0.
    write("proj-b/numbers.jsonl", [
        b'{"type":"assistant","timestamp":"2026-05-04T10:00:00Z","message":{"id":"msg_float","model":"claude-opus-5","usage":{"input_tokens":5.0,"output_tokens":-4,"cache_read_input_tokens":1e3,"cache_creation_input_tokens":12,"cache_creation":{"ephemeral_1h_input_tokens":12}}}}\n',
        b'{"type":"assistant","timestamp":"2026-05-04T10:00:00Z","message":{"id":"msg_zero","model":"claude-opus-5","usage":{"input_tokens":0,"output_tokens":-0,"cache_read_input_tokens":0.0}}}\n',
        b'{"type":"assistant","timestamp":"2026-05-04T10:00:00Z","message":{"id":"msg_big","model":"claude-opus-5","usage":{"input_tokens":123456789012,"output_tokens":1,"weird":[1e400,-1e-400,0.5E+2]}}}\n',
        b'{"type":"assistant","timestamp":"2026-05-04T10:00:00Z","message":{"id":42,"usage":{"input_tokens":5}}}\n',
        b'{"type":"assistant","timestamp":20260504,"message":{"id":"msg_numeric_ts","usage":{"input_tokens":5}}}\n',
        b'{"type":"assistant","timestamp":"2026-05-04T10:00:00Z","message":{"id":"msg_null_model","model":null,"usage":{"input_tokens":5,"cache_creation":null,"server_tool_use":"x"}}}\n',
        b'{"type":"assistant","timestamp":"2026-05-04T10:00:00Z","message":{"id":"msg_true_tokens","model":"claude-opus-5","usage":{"input_tokens":true,"output_tokens":false}}}\n',
    ])

    # Escapes in captured strings and in human text.
    write("proj-c/escapes.jsonl", [
        user("p-esc-1", "2026-05-05T10:00:00Z", "caf\u00e9 \\ \"quoted\" \U0001F600 emoji"),
        b'{"type":"user","uuid":"p-esc-2","timestamp":"2026-05-05T10:00:01Z","message":{"content":"\\u003clocal-command-stdout>x"}}\n',
        b'{"type":"user","uuid":"p-esc-3","timestamp":"2026-05-05T10:00:02Z","message":{"content":"\\n\\t\\r\\f\\u000b "}}\n',
        b'{"type":"user","uuid":"p-esc-4","timestamp":"2026-05-05T10:00:03Z","message":{"content":"\\b"}}\n',
        b'{"type":"user","uuid":"p-esc-5","timestamp":"2026-05-05T10:00:04Z","message":{"content":"\\ud83d lone high"}}\n',
        b'{"type":"user","uuid":"p-esc-6","timestamp":"2026-05-05T10:00:05Z","message":{"content":"\\/\\"\\\\"}}\n',
        b'{"type":"assistant","timestamp":"2026-05-05T10:00:06Z","cwd":"/home/u/caf\\u00e9 \\ud83d\\ude00\\ttab\\nline","sessionId":"s-esc","message":{"id":"msg_esc","model":"claude-sonnet-5","usage":{"input_tokens":1}}}\n',
        b'{"type":"assistant","timestamp":"2026-05-05T10:00:07Z","cwd":"/home/u/lone\\udc00low","sessionId":"s-esc","message":{"id":"msg_esc2","model":"claude-sonnet-5","usage":{"input_tokens":1}}}\n',
        b'{"type":"assistant","timestamp":"2026-05-05T10:00:08Z","message":{"id":"msg_\\u0065sc3","model":"claude-sonnet-\\u0035","usage":{"input_tokens":1},"content":[{"type":"tool_\\u0075se","id":"toolu_esc","name":"W\\u0065bFetch"}]}}\n',
    ])

    # Human prompts: which user records count.
    write("proj-c/prompts.jsonl", [
        user("p-1", "2026-06-01T10:00:00Z", "a real prompt"),
        user("p-1", "2026-06-01T09:00:00Z", "a real prompt, copied earlier"),
        user("p-2", "2026-06-01T10:01:00Z", "meta", meta=True),
        user("p-3", "2026-06-01T10:02:00Z", "from a subagent", side=True),
        user("p-4", "2026-06-01T10:03:00Z", [{"type": "tool_result", "tool_use_id": "t", "content": "out"}]),
        user("p-5", "2026-06-01T10:04:00Z", [{"type": "tool_result", "content": "x"}, {"text": "text before type", "type": "text"}]),
        user("p-6", "2026-06-01T10:05:00Z", [{"type": "text", "text": "[Request interrupted by user]"}]),
        user("p-7", "2026-06-01T10:06:00Z", "<command-name>/model</command-name>"),
        user("p-8", "2026-06-01T10:07:00Z", "<bash-stdout>hi</bash-stdout>"),
        user("p-9", "2026-06-01T10:08:00Z", [{"type": "text", "text": "   "}, {"type": "image", "source": {}}]),
        user("p-10", "2026-06-01T10:09:00Z", [{"type": "text", "text": "ok", "type": "other"}]),
        user("p-11", "2025-12-31T23:30:00-08:00", "new year's eve in LA, new year's day in UTC"),
        {"type": "user", "timestamp": "2026-06-01T10:10:00Z", "message": {"content": "no uuid"}},
        {"type": "user", "uuid": "p-12", "message": {"content": "no timestamp"}},
        user("p-13", "2026-06-01T10:11:00Z", {"not": "a list"}),
    ])

    # Timestamps.
    stamps = [
        "2026-03-08T09:59:59Z", "2026-03-08T10:00:00Z", "2026-11-01T08:59:59Z", "2026-11-01T09:00:00Z",
        "2026-07-01T12:00:00+05:30", "2026-07-01T12:00:00-00:00", "2026-07-01T12:00:00.123456789Z",
        "2026-07-01T12:00:00", "2026-07-01 12:00:00Z", "2026-13-01T12:00:00Z", "2026-02-29T12:00:00Z",
        "2028-02-29T12:00:00Z", "2026-07-01T24:00:00Z", "2026-07-01T12:60:00Z", "2026-07-01T12:00:60Z",
        "2026-07-01T12:00:00z", "2026-07-01T12:00:00+24:00", "2026-07-01T12:00:00+23:59", "2026-07-01T12:00:00.Z",
        "0000-01-01T00:00:00Z", "2025-12-31T23:59:59Z", "2026-01-01T07:59:59Z", "2026-01-01T08:00:00Z",
        "2026-12-31T23:59:59-08:00", "2027-01-01T08:00:00Z", "2026-07-01T12:00:00+05:75", "",
    ]
    write("proj-d/timestamps.jsonl", [assistant(f"msg_ts_{i}", ts) for i, ts in enumerate(stamps)])

    # Pricing.
    write("proj-e/pricing.jsonl", [
        assistant("msg_haiku", "2026-08-01T10:00:00Z", model="claude-haiku-4-5-20251001"),
        assistant("msg_unknown", "2026-08-01T10:00:01Z", model="claude-future-9"),
        assistant("msg_synth", "2026-08-01T10:00:02Z", model="<synthetic>"),
        assistant("msg_fast5", "2026-08-01T10:00:03Z", model="claude-opus-5", u=usage(speed="fast")),
        assistant("msg_fast46", "2026-08-01T10:00:04Z", model="claude-opus-4-6", u=usage(speed="fast")),
        assistant("msg_geo", "2026-08-01T10:00:05Z", model="claude-fable-5-1", u=usage(geo="us", read=1234567)),
        assistant("msg_web", "2026-08-01T10:00:06Z", model="claude-sonnet-4-6", u=usage(ws=3, wf=5)),
        assistant("msg_5555", "2026-08-01T10:00:07Z", model="claude-opus-5-5", u=usage(inp=7, out=11, read=13, write=17, w1h=0, w5m=17)),
        assistant("msg_unsplit", "2026-08-01T10:00:08Z", model="claude-opus-4-8", u=usage(write=100, w1h=0, w5m=0)),
        assistant("msg_over1h", "2026-08-01T10:00:09Z", model="claude-opus-4-8", u=usage(write=100, w1h=150, w5m=0)),
        assistant("msg_dated_bad", "2026-08-01T10:00:10Z", model="claude-opus-5-2026"),
        assistant("msg_sonnet5", "2026-08-01T10:00:11Z", model="claude-sonnet-5", u=usage(inp=1000000, out=1000000, read=1000000, write=1000000)),
        assistant("msg_side", "2026-08-01T10:00:12Z", side=True, session="s-agent"),
    ])

    # Long strings: shortened by the reader when longer than 4096 bytes.
    long_text = "word " * 2000
    write("proj-f/long.jsonl", [
        user("p-long-1", "2026-09-01T10:00:00Z", long_text),
        user("p-long-2", "2026-09-01T10:00:01Z", " " * 5000 + "late ink"),
        user("p-long-3", "2026-09-01T10:00:02Z", " " * 5000),
        user("p-long-4", "2026-09-01T10:00:03Z", "\n" * 3000 + "x"),
        user("p-long-5", "2026-09-01T10:00:04Z", "<local-command-stdout>" + "y" * 6000),
        user("p-long-6", "2026-09-01T10:00:05Z", [{"type": "tool_result", "content": "z" * 9000}, {"type": "text", "text": "\u00e9" * 3000}]),
        b'{"type":"user","uuid":"p-long-7","timestamp":"2026-09-01T10:00:06Z","message":{"content":"' + b"\\ud83d\\ude00" * 800 + b'"}}\n',
        b'{"type":"user","uuid":"p-long-8","timestamp":"2026-09-01T10:00:07Z","message":{"content":"' + b"a" * 5000 + b'\\q"}}\n',
        assistant("msg_longcwd", "2026-09-01T10:00:08Z", cwd="/home/u/" + "d" * 4000),
        assistant("msg_longtool", "2026-09-01T10:00:09Z", content=[tool("toolu_long", "Write", {"content": "c" * 20000})]),
    ])

    # Tools: the least [name, message id] wins; tools of out-of-period responses do not count.
    write("proj-g/tools.jsonl", [
        assistant("msg_t1", "2026-04-01T10:00:00Z", content=[tool("toolu_same", "Read"), tool("toolu_a", "Grep")]),
        assistant("msg_t2", "2026-04-01T10:00:01Z", content=[tool("toolu_same", "Edit")]),
        assistant("msg_t_old", "2025-06-01T10:00:00Z", content=[tool("toolu_old", "Bash")]),
        assistant("msg_t3", "2026-04-01T10:00:02Z", content=[{"type": "tool_use", "name": "NoId"}, {"type": "tool_use", "id": 5, "name": "NumId"}, {"type": "tool_use", "id": "toolu_noname"}]),
        assistant("msg_t4", "2026-04-01T10:00:03Z", content=[{"type": "tool_use", "id": "toolu_mcp", "name": "mcp__server__do"}]),
        {"type": "assistant", "timestamp": "2026-04-01T10:00:04Z", "message": {"content": [tool("toolu_nousage", "Bash")], "id": "msg_nousage"}},
    ])

    # A response streamed across midnight on New Year's Eve in Los Angeles: its first line is in
    # 2025 there, its tool_use line in 2026. The response belongs to 2025, the tool call to 2026.
    write("proj-g/newyear.jsonl", [
        assistant("msg_ny", "2026-01-01T07:59:59.900Z", content=[{"type": "text", "text": "hi"}], u=usage(out=5)),
        assistant("msg_ny", "2026-01-01T08:00:00.100Z", content=[tool("toolu_ny", "Glob")], u=usage(out=50)),
    ])

    # Projects: home itself, under home, outside home, missing cwd, cwd prefix that is not a dir.
    write("proj-h/projects.jsonl", [
        assistant("msg_p1", "2026-10-01T10:00:00Z", cwd=HOME),
        assistant("msg_p2", "2026-10-01T10:00:01Z", cwd=HOME + "/w/x"),
        assistant("msg_p3", "2026-10-01T10:00:02Z", cwd="/srv/other"),
        assistant("msg_p4", "2026-10-01T10:00:03Z", cwd=""),
        assistant("msg_p5", "2026-10-01T10:00:04Z", cwd=HOME + "ish/y"),
        {k: v for k, v in assistant("msg_p6", "2026-10-01T10:00:05Z").items() if k != "cwd"},
        assistant("msg_p7", "2026-10-01T10:00:06Z", cwd=None, sessionId=None),
    ])

    # Names that are markup or terminal control in the formats for people. They must come out
    # as text: escaped in HTML and Markdown, control characters replaced in every format.
    write("proj-h/hostile.jsonl", [
        assistant("msg_h1", "2026-10-03T10:00:00Z", cwd=HOME + "/<script>alert(1)</script>", u=usage(out=40000),
                  content=[tool(f"toolu_h1{i}", "mcp__private-server__<b>x</b>") for i in range(3)]),
        assistant("msg_h2", "2026-10-03T10:00:01Z", cwd=HOME + "/a' onmouseover='x\" &amp; |pipe|", u=usage(out=30000)),
        assistant("msg_h4", "2026-10-03T10:00:03Z", model="<img src=x>"),
        assistant("msg_h3", "2026-10-03T10:00:02Z", cwd=HOME + "/\x1b[31mred\x1b]0;title\x07\x9b", u=usage(out=20000),
                  content=[tool(f"toolu_h3{i}", "mcp__other__read") for i in range(2)]),
    ])

    # Irrelevant structure, deep nesting and noise between relevant fields.
    deep = {"x": [[[[{"y": [1, {"z": [True, False, None, -1.5e-3]}]}]]]], "message": {"id": "shadow"}}
    rec = assistant("msg_deep", "2026-10-02T10:00:00Z")
    rec["attachment"] = deep
    rec["toolUseResult"] = {"stdout": "o" * 5000, "nested": deep}
    write("proj-h/deep.jsonl", [rec, {"type": "progress", "data": deep}, {"type": "system", "content": "x"}])

    # Old files: modified before the year, skipped by the mtime rule.
    write("proj-i/old.jsonl", [assistant("msg_old_file", "2025-03-01T10:00:00Z")])
    old = OUT / "proj-i" / "old.jsonl"
    os.utime(old, (1740000000, 1740000000))

    # A file that is not a transcript and a nested directory.
    (OUT / "proj-i" / "notes.txt").write_text("not jsonl\n")
    write("proj-j/sub/dir/nested.jsonl", [assistant("msg_nested", "2026-11-11T11:11:11Z")])


if __name__ == "__main__":
    main()
