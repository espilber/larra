#!/bin/sh
# Best-effort third-party license dump. Requires cargo-about and pnpm.
# Refreshes the frontend-dependency sentence in THIRD-PARTY-NOTICES.md
# from production packages in package.json.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/dist-licenses"
mkdir -p "$OUT"
if command -v cargo-about >/dev/null 2>&1; then
  (cd "$ROOT/src-tauri" && cargo about generate --output-file "$OUT/cargo-about.html" || cargo about init)
else
  echo "install cargo-about for a full Rust notice: cargo install cargo-about"
fi
(cd "$ROOT" && pnpm licenses list --json > "$OUT/npm-licenses.json")

python3 - "$ROOT" <<'PY'
import json, pathlib, re, sys
root = pathlib.Path(sys.argv[1])
pkg = json.loads((root / "package.json").read_text())
names = sorted(pkg.get("dependencies", {}))
sentence = (
    "Svelte, Vite, Tailwind CSS, "
    + ", ".join(f"`{n}`" if n.startswith("@") else n for n in names)
    + ". Licenses are MIT/Apache-2.0 unless a package README says otherwise."
)
# Keep the human list readable: map package ids to product names where we know them.
pretty = {
    "@lucide/svelte": "Lucide",
    "@tauri-apps/api": "`@tauri-apps/api`",
    "dompurify": "DOMPurify",
    "marked": "marked",
    "svelte-sonner": "svelte-sonner",
}
shown = ["Svelte", "Vite", "Tailwind CSS"] + [pretty.get(n, n) for n in names]
sentence = (
    ", ".join(shown)
    + ". Licenses are MIT/Apache-2.0 unless a package README says otherwise."
)
notices = root / "THIRD-PARTY-NOTICES.md"
text = notices.read_text()
updated, n = re.subn(
    r"(## Direct frontend dependencies\n\n)(.*?)(\n\n## )",
    r"\1" + sentence + r"\3",
    text,
    count=1,
    flags=re.S,
)
if n != 1:
    raise SystemExit("could not find Direct frontend dependencies section")
notices.write_text(updated)
print("updated THIRD-PARTY-NOTICES.md frontend list")
PY

echo "Wrote $OUT"
