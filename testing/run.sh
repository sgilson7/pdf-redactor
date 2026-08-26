#!/usr/bin/env bash
# Browser-level checks: drives the built app in headless Chromium, screenshots
# each stage, fails on any console error or off-origin request, and verifies
# each exported PDF with an independent implementation (poppler).
#
# The redaction engine is covered by `cargo test`. This covers the part that
# is not: the UI that decides what the engine is asked to do.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PY="$ROOT/.venv-test/bin/python"

[ -x "$PY" ] || { echo "run: make test-ui-setup"; exit 1; }

"$PY" "$ROOT/testing/make_fixtures.py" >/dev/null
"$ROOT/packaging/package-web.sh" >/dev/null

fail=0
run() { echo; echo "── $1 ─────────────────────────"; shift; "$PY" "$ROOT/testing/drive.py" "$@" 2>&1 \
  | grep -vE '^127\.0\.0\.1' || fail=1; }

run "typed homework"  --tag homework --pdf homework.pdf --export \
    --manifest --forbid "Jane,Doe,jdoe2,ncsu,homework.pdf" --expect-no-ocr-assets
run "gemini chatlog"  --tag chatlog  --pdf chatlog.pdf  --export
run "wrapped lines"   --tag wrapped  --pdf wrapped.pdf  --extras "" --export
run "author block"    --tag affil    --pdf affil.pdf    --extras "" --export \
    --manifest --forbid "Jane,Doe,btaghiz,ncsu,affil.pdf"
run "screenshot page" --tag shot     --pdf screenshot.pdf --extras "" --scan-page --export
run "image-only scan" --tag scanned  --pdf scanned.pdf  --extras "" --draw --export

echo
echo "── adversarial checks on exported PDFs ──"
# What must be absent depends on what each run actually approved. A declined
# term legitimately survives -- the affiliation fixture leaves its address
# unchecked on purpose -- so a single blanket pattern would fail a correct file.
must_be_absent() {
  case "$1" in
    affil.pdf)   echo 'jane doe|Microsoft Word|Acme PDF' ;;
    shot.pdf)    echo 'jane doe|jdoe2' ;;
    wrapped.pdf) echo 'jane doe|Microsoft Word|Acme PDF' ;;
    *)           echo 'jane doe|jdoe2|ncsu\.edu|Microsoft Word|Acme PDF' ;;
  esac
}
for f in "$ROOT"/testing/out/*.pdf; do
  n=$(basename "$f")
  leak=$(strings "$f" | grep -icE "$(must_be_absent "$n")" || true)
  meta=$(strings "$f" | grep -cE '/Info|/Author|/Producer|/Metadata|/Annots' || true)
  revs=$(strings "$f" | grep -c '%%EOF' || true)
  printf "  %-16s approved-terms:%s metadata:%s revisions:%s" "$n" "$leak" "$meta" "$revs"
  if [ "$leak" = 0 ] && [ "$meta" = 0 ] && [ "$revs" = 1 ]; then echo "  ✓"; else echo "  ✗"; fail=1; fi
done

echo
[ "$fail" = 0 ] && echo "all browser checks passed" || { echo "FAILURES"; exit 1; }
