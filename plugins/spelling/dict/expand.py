import re, sys, pathlib

def load_aff(path):
    rules = {}
    lines = pathlib.Path(path).read_text(encoding="utf-8").splitlines()
    i = 0
    while i < len(lines):
        parts = lines[i].split()
        if len(parts) == 4 and parts[0] in ("PFX", "SFX") and parts[2] in ("Y", "N"):
            kind, flag, cross, n = parts[0], parts[1], parts[2] == "Y", int(parts[3])
            entries = []
            for line in lines[i + 1 : i + 1 + n]:
                p = line.split()
                strip = "" if p[2] == "0" else p[2]
                add = "" if p[3] == "0" else p[3].split("/")[0]
                cond = p[4] if len(p) > 4 else "."
                rx = re.compile(("^" + cond) if kind == "PFX" else (cond + "$"))
                entries.append((strip, add, rx))
            rules[flag] = (kind, cross, entries)
            i += n + 1
        else:
            i += 1
    return rules

def apply(word, rule):
    kind, _, entries = rule
    out = []
    for strip, add, rx in entries:
        if not rx.search(word):
            continue
        if kind == "SFX":
            if strip and not word.endswith(strip):
                continue
            out.append(word[: len(word) - len(strip)] + add)
        else:
            if strip and not word.startswith(strip):
                continue
            out.append(add + word[len(strip) :])
    return out

def main(aff, dic, out):
    rules = load_aff(aff)
    words = set()
    for line in pathlib.Path(dic).read_text(encoding="utf-8").splitlines()[1:]:
        word, _, flags = line.partition("/")
        word = word.strip()
        if not word or any(c.isdigit() for c in word):
            continue
        words.add(word)
        sfx = [rules[f] for f in flags if f in rules and rules[f][0] == "SFX"]
        pfx = [rules[f] for f in flags if f in rules and rules[f][0] == "PFX"]
        suffixed = []
        for r in sfx:
            suffixed += [(w, r[1]) for w in apply(word, r)]
        words.update(w for w, _ in suffixed)
        for p in pfx:
            words.update(apply(word, p))
            if p[1]:
                for w, cross in suffixed:
                    if cross:
                        words.update(apply(w, p))
    words = sorted(words)
    pathlib.Path(out).write_text("\n".join(sorted(words, key=lambda w: w.encode())) + "\n", encoding="utf-8")
    print(len(words), "words")

main(*sys.argv[1:4])
