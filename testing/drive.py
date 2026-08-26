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
    ap.add_argument("--draw", action="store_true",
                    help="drag a manual redaction box on page 1")
    ap.add_argument("--headed", action="store_true")
    args = ap.parse_args()

    SHOTS.mkdir(exist_ok=True)
    httpd = serve()
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
        page.on("request", lambda r: problems.append(f"OFF-ORIGIN REQUEST: {r.url}")
                if not r.url.startswith((f"http://127.0.0.1:{PORT}", "data:", "blob:")) else None)

        def shot(n):
            page.screenshot(path=str(SHOTS / f"{args.tag}-{n}.png"))
            print(f"  shot: {args.tag}-{n}.png")

        print(f"→ loading http://127.0.0.1:{PORT}/")
        page.goto(f"http://127.0.0.1:{PORT}/", wait_until="networkidle")
        page.wait_for_timeout(900)
        shot("1-landing")

        # Layout assertions: exactly the drop zone, nothing stacked behind it.
        for sel, want in [("#drop", True), ("#app", False), ("#modal", False), ("#busy", False)]:
            vis = page.locator(sel).is_visible()
            if vis != want:
                problems.append(f"on load, {sel} visible={vis}, expected {want}")
        print(f"  landing visibility ok" if not problems else "  landing has problems")

        print(f"→ uploading {args.pdf}")
        page.set_input_files("#file", str(FIX / args.pdf))
        page.wait_for_selector("#app:visible", timeout=15000)
        page.wait_for_selector("#busy", state="hidden", timeout=30000)
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

        if args.draw:
            print("→ drawing a manual box")
            ov = page.locator("#overlay")
            box = ov.bounding_box()
            page.mouse.move(box["x"] + 60, box["y"] + 100)
            page.mouse.down()
            page.mouse.move(box["x"] + 260, box["y"] + 130, steps=8)
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
            page.wait_for_selector("#modal:visible", timeout=60000)
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

        b.close()
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
