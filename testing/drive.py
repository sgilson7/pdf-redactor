#!/usr/bin/env python3
"""Drive the built app in a real browser: screenshot it, capture console
errors, and run the full upload -> find -> review -> export flow.

Exists because the redaction engine is testable under `cargo test` but the UI
is not, and a front-end bug can silently defeat the whole guarantee.
"""
import sys, pathlib, http.server, socketserver, threading, functools, argparse
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parent
DIST = ROOT.parent / "dist" / "web"
SHOTS = ROOT / "shots"
FIX = ROOT / "fixtures"
PORT = 8123


def serve():
    handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(DIST))
    socketserver.TCPServer.allow_reuse_address = True
    httpd = socketserver.TCPServer(("127.0.0.1", PORT), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--pdf", default="homework.pdf")
    ap.add_argument("--name", default="Jane Doe")
    ap.add_argument("--extras", default="jdoe2@ncsu.edu")
    ap.add_argument("--tag", default="run")
    ap.add_argument("--export", action="store_true")
    ap.add_argument("--expect-none", action="store_true",
                    help="image-only documents have nothing to search")
    ap.add_argument("--manifest", action="store_true",
                    help="check the recorded manifest after export")
    ap.add_argument("--forbid", default="",
                    help="comma-separated words that must not appear in the manifest")
    ap.add_argument("--approve-all", action="store_true",
                    help="tick every candidate, including medium and low")
    ap.add_argument("--pager", action="store_true",
                    help="exercise page navigation")
    ap.add_argument("--draw", action="store_true",
                    help="drag a manual redaction box on page 1")
    ap.add_argument("--headed", action="store_true")
    ap.add_argument("--url", default=None,
                    help="test a deployed site instead of the local build")
    args = ap.parse_args()

    SHOTS.mkdir(exist_ok=True)
    base = args.url.rstrip("/") if args.url else f"http://127.0.0.1:{PORT}"
    httpd = None if args.url else serve()
    problems, logs = [], []

    with sync_playwright() as p:
        b = p.chromium.launch(headless=not args.headed)
        page = b.new_page(viewport={"width": 1440, "height": 900})

        page.on("console", lambda m: (
            logs.append(f"[{m.type}] {m.text}"),
            problems.append(f"console.{m.type}: {m.text}") if m.type == "error" else None))
        page.on("pageerror", lambda e: problems.append(f"pageerror: {e}"))
        # Nothing should ever be requested off-origin. This is the privacy
        # claim, asserted rather than assumed.
        # Nothing may be fetched from anywhere but the app's own origin. On the
        # deployed site this is the privacy claim itself, checked rather than
        # asserted.
        page.on("request", lambda r: problems.append(f"OFF-ORIGIN REQUEST: {r.url}")
                if not r.url.startswith((base, "data:", "blob:")) else None)

        def shot(n):
            page.screenshot(path=str(SHOTS / f"{args.tag}-{n}.png"))
            print(f"  shot: {args.tag}-{n}.png")

        print(f"→ loading {base}/")
        page.goto(f"{base}/", wait_until="networkidle")
        page.wait_for_timeout(900)
        shot("1-landing")

        # Layout assertions: exactly the drop zone, nothing stacked behind it.
        for sel, want in [("#drop", True), ("#app", False), ("#modal", False), ("#busy", False)]:
            vis = page.locator(sel).is_visible()
            if vis != want:
                problems.append(f"on load, {sel} visible={vis}, expected {want}")
        print(f"  landing visibility ok" if not problems else "  landing has problems")

        print(f"→ uploading {args.pdf}")
        src = pathlib.Path(args.pdf)
        if not src.is_absolute():
            src = FIX / args.pdf
        page.set_input_files("#file", str(src))
        page.wait_for_selector("#app:visible", timeout=15000)
        page.wait_for_selector("#busy", state="hidden", timeout=180000)
        page.wait_for_timeout(600)
        shot("2-opened")

        pages = page.locator("#rail .thumb").count()
        print(f"  rendered {pages} page thumbnail(s)")
        if pages == 0:
            problems.append("no page thumbnails rendered")

        print(f"→ searching for {args.name!r}")
        page.fill("#name", args.name)
        page.fill("#extras", args.extras)
        page.click("#scan")
        page.wait_for_timeout(900)
        shot("3-found")

        hits = page.locator("#list .hit").count()
        boxes = page.locator("#overlay .rbox").count()
        checked = page.locator("#list .hit input:checked").count()
        print(f"  {hits} hit(s), {checked} pre-checked, {boxes} box(es) drawn on page 1")
        if hits == 0 and not args.expect_none:
            problems.append(f"no matches found for {args.name!r}")
        if hits and args.expect_none:
            problems.append(f"expected no matches but found {hits}")

        for row in page.locator("#list .hit").all()[:8]:
            t = row.locator(".tier").inner_text() if row.locator(".tier").count() else "?"
            m = row.locator(".m").inner_text() if row.locator(".m").count() else "?"
            ck = row.locator("input").is_checked() if row.locator("input").count() else False
            print(f"    [{'x' if ck else ' '}] {t:7} {m}")

        if args.approve_all:
            n = page.evaluate("""() => {
              const S = window.__redactor.state;
              S.hits.forEach(h => h.on = true);
              return S.hits.length;
            }""")
            page.click("#scan") if False else None
            page.evaluate("() => { document.getElementById('export').disabled = false; }")
            # Re-render so boxes and counts reflect the change.
            page.evaluate("() => window.dispatchEvent(new Event('resize'))")
            print(f"  approved all {n} candidates")

        if args.pager:
            print("→ exercising the pager")
            total = int(page.locator("#pagetotal").inner_text())
            page.click("#next"); page.wait_for_timeout(350)
            after_next = page.locator("#pageno").input_value()
            page.click("#last"); page.wait_for_timeout(700)
            at_last = page.locator("#pageno").input_value()
            nxt_disabled = page.locator("#next").is_disabled()
            page.fill("#pageno", "42"); page.press("#pageno", "Enter")
            page.wait_for_timeout(600)
            jumped = page.locator("#pageno").input_value()
            # Blur the number field: arrow keys are ignored while typing.
            page.evaluate("document.activeElement?.blur()")
            page.keyboard.press("ArrowLeft"); page.wait_for_timeout(600)
            after_key = page.locator("#pageno").input_value()
            print(f"  total={total} next={after_next} last={at_last} "
                  f"(next disabled: {nxt_disabled}) jump={jumped} arrowleft={after_key}")
            for want, got, what in [("2", after_next, "next"), (str(total), at_last, "last"),
                                    ("42", jumped, "jump"), ("41", after_key, "arrow key")]:
                if want != got:
                    problems.append(f"pager {what}: expected {want}, got {got}")
            if not nxt_disabled:
                problems.append("next not disabled on last page")
            shot("3c-pager")

        if args.draw:
            print("→ drawing a manual box")
            ov = page.locator("#overlay")
            ov.scroll_into_view_if_needed()
            page.wait_for_timeout(250)
            box = ov.bounding_box()
            # A tall page can extend past the viewport, which puts a naive
            # offset off-screen and the drag never lands.
            vh = page.viewport_size["height"]
            y = max(box["y"], 0) + min(100, max(40, (min(box["y"] + box["height"], vh)
                                                      - max(box["y"], 0)) / 3))
            page.mouse.move(box["x"] + 60, y)
            page.mouse.down()
            page.mouse.move(box["x"] + 260, y + 30, steps=8)
            page.mouse.up()
            page.wait_for_timeout(400)
            drawn = page.locator("#overlay .rbox").count()
            manual = page.locator("#list .tier.manual").count()
            print(f"  {drawn} box(es) on page, {manual} manual entr(y/ies) listed")
            if manual == 0:
                problems.append("manual box did not register in the review list")
            shot("3b-manual")

        if args.export:
            print("→ exporting")
            page.click("#export")
            page.wait_for_selector("#modal:visible", timeout=600000)
            page.wait_for_timeout(500)
            shot("4-report")
            print("  " + page.locator("#mtitle").inner_text())
            for line in page.locator("#mbody .chk").all():
                print("    " + line.inner_text().replace("\n", " "))

            dl_visible = page.locator("#mdownload").is_visible()
            if dl_visible:
                with page.expect_download() as di:
                    page.click("#mdownload")
                out = ROOT / "out" / f"{args.tag}.pdf"
                out.parent.mkdir(exist_ok=True)
                di.value.save_as(str(out))
                print(f"  downloaded -> {out}")
            else:
                problems.append("verification failed; download blocked")

        if args.manifest:
            print("→ checking the manifest")
            n = page.evaluate("() => JSON.parse(localStorage.getItem('pdf-redactor.manifest.v1')||'[]').length")
            print(f"  entries recorded: {n}")
            if n < 1:
                problems.append("no manifest entry was recorded after export")
            js = page.evaluate("""() => {
              const e = JSON.parse(localStorage.getItem('pdf-redactor.manifest.v1')||'[]');
              return window.__redactor.buildManifest(
                document.getElementById('build').textContent.trim(), JSON.stringify(e));
            }""")
            out = ROOT / "out" / f"{args.tag}-manifest.json"
            out.parent.mkdir(exist_ok=True)
            out.write_text(js)
            print(f"  wrote {out.name} ({len(js)} bytes)")
            import json as _j
            m = _j.loads(js)
            if m.get("containsPersonalData") is not False:
                problems.append("manifest does not declare containsPersonalData=false")
            # The browser-level twin of the Rust guard test.
            for word in args.forbid.split(",") if args.forbid else []:
                if word and word.lower() in js.lower():
                    problems.append(f"manifest leaked {word!r}")
            print(f"  schema={m.get('schema')} docs={len(m.get('documents',[]))} "
                  f"containsPersonalData={m.get('containsPersonalData')}")

        b.close()
    if httpd:
        httpd.shutdown()

    print("\n=== console ===")
    for l in logs[-12:]:
        print("  " + l)
    print("\n=== problems ===")
    if problems:
        for p_ in problems:
            print("  ✗ " + p_)
        sys.exit(1)
    print("  none")


main()
