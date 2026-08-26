#!/usr/bin/env bash
# Build the browser app into dist/web/.
#
# No node, no npm, no bundler: the page is hand-written ES modules, pdf.js is
# vendored, and wasm-bindgen emits the only generated file.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"; WEB="$DIST/web"
CRATE=redactor-wasm
WASM=redactor_wasm
TARGET=wasm32-unknown-unknown

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

rustup target list --installed 2>/dev/null | grep -q "^$TARGET$" \
  || die "missing target. Run: rustup target add $TARGET"

# wasm-bindgen-cli and the wasm-bindgen crate must be the same version or the
# generated glue will not match the module. Read the version the lockfile
# actually pinned rather than trusting whatever happens to be installed.
NEED=$(awk '/^name = "wasm-bindgen"$/{f=1} f&&/^version = /{gsub(/"/,"");print $3;exit}' "$ROOT/Cargo.lock")
command -v wasm-bindgen >/dev/null \
  || die "wasm-bindgen not found. Run: cargo install wasm-bindgen-cli --version $NEED"
HAVE=$(wasm-bindgen --version | awk '{print $2}')
[ "$HAVE" = "$NEED" ] \
  || die "wasm-bindgen $HAVE installed but the lockfile pins $NEED.
  Run: cargo install wasm-bindgen-cli --version $NEED --force"

say "Building $CRATE for $TARGET"
cargo build --release --target "$TARGET" -p "$CRATE"

say "Assembling $WEB"
rm -rf "$WEB"; mkdir -p "$WEB"
cp -R "$ROOT/web/." "$WEB/"

say "Generating JS bindings"
wasm-bindgen --target web --no-typescript \
  --out-dir "$WEB/pkg" \
  "$ROOT/target/$TARGET/release/$WASM.wasm"

# wasm-opt is a nice-to-have, not a requirement.
if command -v wasm-opt >/dev/null; then
  say "Optimising wasm"
  wasm-opt -Oz -o "$WEB/pkg/${WASM}_bg.wasm" "$WEB/pkg/${WASM}_bg.wasm"
fi

# --- cache busting -----------------------------------------------------------
# GitHub Pages serves everything with `Cache-Control: max-age=600`, so for ten
# minutes after a deploy a reload keeps using the previous app.js and .wasm from
# disk cache. That reads as "the fix did not deploy", and worse, a browser can
# mix a fresh script with a stale module. Stamping the content hash into every
# internal asset URL means a changed build is simply a different URL, so a
# stale copy can never be served for it.
say "Stamping build version"
BUILD=$(cat "$WEB/app.js" "$WEB/pkg/$WASM.js" "$WEB/pkg/${WASM}_bg.wasm" \
        | shasum -a 256 | cut -c1-8)

# app.js -> pkg/redactor_wasm.js -> redactor_wasm_bg.wasm: every hop needs it,
# because the browser caches each URL independently.
sed -i '' "s|from './pkg/$WASM.js'|from './pkg/$WASM.js?v=$BUILD'|" "$WEB/app.js"
sed -i '' "s|new URL('${WASM}_bg.wasm', import.meta.url)|new URL('${WASM}_bg.wasm?v=$BUILD', import.meta.url)|" \
  "$WEB/pkg/$WASM.js"
sed -i '' "s|src=\"app.js\"|src=\"app.js?v=$BUILD\"|; s|href=\"styles.css\"|href=\"styles.css?v=$BUILD\"|" \
  "$WEB/index.html"
sed -i '' "s|__BUILD__|$BUILD|" "$WEB/index.html"

# Jekyll would otherwise skip the pkg/ directory and mangle assets.
touch "$WEB/.nojekyll"

say "Done: $WEB (build $BUILD)"
du -sh "$WEB" | sed 's/^/    /'
ls -la "$WEB/pkg/${WASM}_bg.wasm" | awk '{printf "    wasm: %.0f KB\n", $5/1024}'
