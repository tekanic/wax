#!/bin/zsh
set -euo pipefail
cd "$(dirname "$0")/.."
command -v pandoc >/dev/null || { echo "build-site.sh needs pandoc (brew install pandoc)"; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

python3 - "$tmp" <<'EOF'
import re, sys, pathlib
out = pathlib.Path(sys.argv[1])
gh = "https://github.com/tekanic/wax"

def links(text, to_repo):
    text = re.sub(r"\]\(docs/plugins\.md(#[^)]*)?\)", lambda m: "](../plugins/" + (m.group(1) or "") + ")", text)
    def repo(m):
        path = m.group(1)
        kind = "blob" if "." in path.rsplit("/", 1)[-1] else "tree"
        return f"]({gh}/{kind}/main/{path})"
    return re.sub(to_repo, repo, text)

readme = pathlib.Path("README.md").read_text()
sections = {}
for part in re.split(r"(?m)^(?=## )", readme):
    m = re.match(r"## (.+)\n", part)
    if m:
        sections[m.group(1).strip()] = part
order = ["Install", "Accounts", "Keyboard", "Labels", "Plugins", "Themes", "Your files", "Problems"]
guide = "\n".join(sections[name] for name in order if name in sections)
guide = links(guide, r"\]\(((?:LICENSE|wit|plugins|src|scripts|assets)[^)#]*)\)")
(out / "guide.md").write_text(guide)

plugins = pathlib.Path("docs/plugins.md").read_text()
plugins = re.sub(r"\A# .*\n+(?:[^\n]+\n)+\n*", "", plugins)
plugins = links(plugins, r"\]\(\.\./([^)#]+)\)")
(out / "plugins.md").write_text(plugins)
EOF

render() {
  local src=$1 slug=$2 title=$3 desc=$4 source=$5
  pandoc "$src" --from gfm+attributes --to html5 --standalone --toc --toc-depth=2 \
    --template docs/src/template.html --highlight-style breezedark \
    -M pagetitle="$title" -M description="$desc" -M slug="$slug" -M source="$source" -M "$slug=true" \
    -o "docs/$slug/index.html"
}

render "$tmp/guide.md" guide "User guide" "Install Wax, sign in with Google, and learn the keys, labels, plugins, themes and settings files." README.md
render "$tmp/plugins.md" plugins "Writing a plugin" "Wax plugins are sandboxed WebAssembly components. This guide goes from an empty folder to a working plugin, then covers the whole interface." docs/plugins.md
echo "built docs/guide/index.html and docs/plugins/index.html"
