"""Collects the English UI text: HTML, the UI scripts and Rust messages that reach the UI.
Writes ui/lang/en.json: every string or template ({0}, {1} for the parts filled in at run time)."""
import json, re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UI = ROOT / "ui"
SKIP_JS = {"mock.js", "fans-mock.js", "audio-mock.js", "headset-mock.js", "i18n.js", "logs.js"}
# Files whose short capitalised strings are labels shown in the UI: effects, settings, presets.
LABELS = {"effect.rs", "post.rs", "proto.rs", "turtlebeach.rs", "profiles.rs", "presets.rs", "dsp.rs", "chain.rs", "razer.rs"}
RUST = [ROOT / "src-tauri" / "src", *sorted((ROOT / "crates").glob("*/src"))]

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

class Html(__import__("html.parser").parser.HTMLParser):
    def __init__(self, name):
        super().__init__(); self.name = name; self.skip = 0
    def handle_starttag(self, tag, attrs):
        if tag in ("script", "style"): self.skip += 1
        for k, v in attrs:
            if k in ("title", "aria-label", "placeholder", "alt", "data-hint") and v: add(v, self.name)
    def handle_endtag(self, tag):
        if tag in ("script", "style"): self.skip -= 1
    def handle_data(self, d):
        if not self.skip: add(d, self.name)

for f in ("index.html", "logs.html"):
    Html(f).feed((UI / f).read_text(encoding="utf-8"))

LIT = re.compile(r'"(?:[^"\\]|\\.)*"' r"|'(?:[^'\\]|\\.)*'" r"|`(?:[^`\\]|\\.)*`")

def unq(lit):
    body = lit[1:-1]
    if lit[0] == "`":
        body = re.sub(r"\$\{[^}]*\}", "\x02", body)
    return body.replace('\\"', '"').replace("\\'", "'").replace("\\n", " ")

def number(chain):
    n = iter(range(99))
    return re.sub("\x02", lambda m: "{%d}" % next(n), chain)

for f in sorted(UI.glob("*.js")):
    if f.name in SKIP_JS: continue
    for line in f.read_text(encoding="utf-8").splitlines():
        if line.strip().startswith("//"): continue
        lits = list(LIT.finditer(line))
        i = 0
        while i < len(lits):
            chain = ("\x02" if re.search(r"[\w)\]]\s*\+\s*$", line[:lits[i].start()]) else "") + unq(lits[i].group())
            j = i
            while j + 1 < len(lits):
                between = line[lits[j].end():lits[j + 1].start()]
                if re.fullmatch(r"\s*\+\s*", between):
                    chain += unq(lits[j + 1].group())
                elif re.fullmatch(r"\s*\+\s*[^\"'`,;]+?\s*\+\s*", between):
                    chain += "\x02" + unq(lits[j + 1].group())
                else:
                    break
                j += 1
            if re.match(r"\s*\+\s*[\w(]", line[lits[j].end():]):
                chain += "\x02"
            add(number(chain), f.name)
            i = j + 1

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

for d in RUST:
    for f in sorted(d.rglob("*.rs")):
        text = strip_test_mods(f.read_text(encoding="utf-8"))
        for m in re.finditer(r'"((?:[^"\\]|\\.)*)"', text):
            s = m.group(1)
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
            elif re.match(r"[A-Z]", s) and " " in s and plain and not re.search(r"::|\\|[a-z]+_[a-z]+\b|\$|%[0-9a-z]", s):
                add(s, f.name)

# Built at run time from ids or option tables the scan above can't see.
RUNTIME = ["Left", "Right", "Custom {0}", "Preset {0}", "TalkThru", "Transparency", "Competition",
           "Immersive", "Night", "Streaming", "Tournament", "Treble Boost"]
for s in RUNTIME:
    add(s, "runtime")

out = UI / "lang" / "en.json"
out.parent.mkdir(exist_ok=True)
data = {k: k for k in sorted(found)}
out.write_text(json.dumps(data, ensure_ascii=False, indent=1) + "\n", encoding="utf-8", newline="\n")
print(len(data), "strings")
