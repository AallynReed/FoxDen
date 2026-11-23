"""Collects the English UI text: the Slint files and Rust messages that reach the UI.
Writes native/lang/en.json: every string or template ({0}, {1} for the parts filled in at run time)."""
import json, re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UI = ROOT / "native" / "ui"
# Files whose short capitalised strings are labels shown in the UI: effects, settings, presets.
LABELS = {"effect.rs", "post.rs", "proto.rs", "turtlebeach.rs", "profiles.rs", "presets.rs", "dsp.rs", "chain.rs", "razer.rs"}
# Text for AI assistants, never shown in the window.
NOT_UI = {ROOT / "crates" / "foxden-mcp" / "src", ROOT / "native" / "src" / "mcp.rs", ROOT / "native" / "src" / "fan_proposal.rs"}
RUST = [ROOT / "native" / "src", *(d for d in sorted((ROOT / "crates").glob("*/src")) if d not in NOT_UI)]

def wordy(s):
    s = s.strip()
    words = s.split()
    if all(re.fullmatch(r"[a-z0-9-]+", w) for w in words) and any("-" in w for w in words):
        return False
    if len(s) < 2 or not re.search(r"[A-Za-z]{2}", s):
        return False
    if re.fullmatch(r"[a-z0-9_.:\-/#\[\]=\"'()*>~^$|]+", s):  # ids, classes, selectors, commands
        return False
    if re.search(r"[{}<>;]|=>|\\|https?://|\.(js|css|png|svg|json|exe|dll|bin)\b", s.replace("{0}", "").replace("{1}", "").replace("{2}", "").replace("{3}", "")):
        return False
    return bool(re.match(r"[A-Z0-9\"'(…{]", s)) or " " in s

found = {}

def add(s, where):
    s = re.sub(r"\s+", " ", s).strip()
    if wordy(s):
        found.setdefault(s, where)

LIT = re.compile(r'"(?:[^"\\]|\\.)*"')

def unq(lit):
    body = re.sub(r"\\\{[^}]*\}", "\x02", lit[1:-1])
    return body.replace('\\"', '"').replace("\\n", " ")

def number(chain):
    n = iter(range(99))
    return re.sub("\x02", lambda m: "{%d}" % next(n), chain)

def top_level(expr, sep):
    """Splits `expr` at `sep` outside strings and brackets."""
    parts, depth, i, start = [], 0, 0, 0
    while i < len(expr):
        if m := LIT.match(expr, i):
            i = m.end()
            continue
        c = expr[i]
        depth += c in "([" and 1 or c in ")]" and -1 or 0
        if depth == 0 and c == sep:
            parts.append(expr[start:i]); start = i + 1
        i += 1
    return parts + [expr[start:]]

def call_args(text, at):
    """The text inside the brackets that open at `at`."""
    depth, i = 0, at
    while i < len(text):
        if m := LIT.match(text, i):
            i = m.end()
            continue
        depth += {"(": 1, ")": -1}.get(text[i], 0)
        i += 1
        if depth == 0:
            return text[at + 1:i - 1]
    return ""

def template(expr):
    """`"Stage " + (i + 1) + " level"` gives "Stage {0} level"; a choice gives each side."""
    if "?" in "".join(top_level(expr, "+")) and len(top_level(expr, "+")) == 1:
        for lit in LIT.findall(expr):
            add(unq(lit), "slint")
        return
    chain = ""
    for part in top_level(expr, "+"):
        part = part.strip()
        chain += unq(part) if LIT.fullmatch(part) else "\x02"
    if chain.replace("\x02", "").strip():
        add(number(chain), "slint")

for f in sorted(UI.glob("*.slint")):
    text = re.sub(r"//[^\n]*", "", f.read_text(encoding="utf-8"))
    for m in re.finditer(r"\bI18n\.t\(", text):
        template(call_args(text, m.end() - 1))
    # Labels handed to widgets that translate them, such as `label: "Speed"`.
    for m in LIT.finditer(text):
        s = unq(m.group())
        words = [w for w in re.findall(r"[A-Za-z]+", s) if w.islower() and len(w) >= 3]
        if "\x02" not in s and s == s.strip() and (re.fullmatch(r"[A-Z][a-z]+", s) or (re.match(r"[A-Z]", s) and " " in s and words)):
            add(s, f.name)

TEST_MOD = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*([{;])")
SKIP = re.compile(r'r(#*)"[\s\S]*?"\1|"(?:[^"\\]|\\.)*"|\'(?:\\.|[^\\\'])\'|//[^\n]*|/\*[\s\S]*?\*/')

def strip_test_mods(text):
    """Drops `#[cfg(test)]` modules, braces balanced outside strings, chars and comments."""
    out, pos = [], 0
    while m := TEST_MOD.search(text, pos):
        out.append(text[pos:m.start()])
        i, depth = m.end(), 1 if m.group(1) == "{" else 0
        while depth and i < len(text):
            if s := SKIP.match(text, i):
                i = s.end()
                continue
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            i += 1
        pos = i
    return "".join(out) + text[pos:]

def rust_strings():
    """Each string literal outside tests and comments, with its file."""
    for d in RUST:
        for f in sorted(d.rglob("*.rs")):
            if f.name == "tests.rs" or "tests" in f.parts or f in NOT_UI:
                continue
            text, i = strip_test_mods(f.read_text(encoding="utf-8")), 0
            while i < len(text):
                m = SKIP.match(text, i)
                if not m:
                    i += 1
                    continue
                if m.group().startswith('"'):
                    yield f, m.group()[1:-1]
                i = m.end()

strings = list(rust_strings())
for f, s in strings:
    if "{" in s:
        n = iter(range(99))
        s = s.replace("{{", "\x00").replace("}}", "\x01")
        s = re.sub(r"\{[^{}]*\}", lambda _: "{%d}" % next(n), s).replace("\x00", "{").replace("\x01", "}")
    s = s.replace('\\"', '"').replace("\\n", " ")
    # Messages people read: a capital start, a space, and plain words, which product names lack.
    plain = [w for w in re.findall(r"[A-Za-z]+", s) if w.islower() and len(w) >= 3]
    label = f.name in LABELS and re.fullmatch(r"[A-Z][a-z]+( [A-Za-z]+){0,3}", s)
    if label:
        add(s, f.name)
    elif re.match(r"[A-Z]|\{\d\}: [a-z]", s) and " " in s and plain and not re.search(r"::|\\|[a-z]+_[a-z]+\b|\$|%[0-9a-z]", s):
        add(s, f.name)

# Whatever the app passes to its translate calls, short lowercase words included.
for f in sorted((ROOT / "native" / "src").rglob("*.rs")):
    text = strip_test_mods(f.read_text(encoding="utf-8"))
    for m in re.finditer(r'\b(?:tr|tf|tell)\(\s*[\w&.()]+\s*,\s*&?"((?:[^"\\]|\\.)*)"', text):
        s = m.group(1).replace('\\"', '"')
        n = iter(range(99))
        s = re.sub(r"\{[^{}]*\}", lambda _: "{%d}" % next(n), s)
        if s == s.strip() and re.search(r"[A-Za-z]{2}", s):
            found.setdefault(s, f.name)

# A string kept from the last run while its text is still in the code, such as a label in a table.
corpus = "\n".join('"' + s + '"' for _, s in strings)
corpus += "\n".join(f.read_text(encoding="utf-8") for f in UI.glob("*.slint"))
old = ROOT / "native" / "lang" / "en.json"
for s in json.loads(old.read_text(encoding="utf-8")) if old.exists() else []:
    if s in found or re.fullmatch(r"[MmLlHhVvCcSsQqAaZz0-9 .,\-]+", s):
        continue
    pieces = [p.replace('"', '\\"') for p in re.split(r"\{\d\}", s) if p.strip()]
    whole = '"' + s.replace('"', '\\"') + '"'
    if (whole in corpus if "{0}" not in s else pieces and all(p in corpus for p in pieces)):
        add(s, "kept")

# Built at run time from ids or option tables the scan above can't see.
RUNTIME = ["Left", "Right", "Custom {0}", "Preset {0}", "TalkThru", "Transparency", "Competition",
           "Immersive", "Night", "Streaming", "Tournament", "Treble Boost",
           "Lowest is {0} at {1}%, charging", "Now {0}%, charging"]
for s in RUNTIME:
    add(s, "runtime")

out = ROOT / "native" / "lang" / "en.json"
out.parent.mkdir(exist_ok=True)
data = {k: k for k in sorted(found)}
out.write_text(json.dumps(data, ensure_ascii=False, indent=1) + "\n", encoding="utf-8", newline="\n")
print(len(data), "strings")
