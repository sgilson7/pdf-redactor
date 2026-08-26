#!/usr/bin/env python3
"""Generate stand-in PDFs for the three document shapes this tool has to handle.

These are not a substitute for real submissions, but they exercise the paths
that differ: a text-layer document, one where the name is only pixels, and a
multi-page mix.
"""
import zlib, struct, pathlib
from PIL import Image, ImageDraw

OUT = pathlib.Path(__file__).parent / "fixtures"
OUT.mkdir(exist_ok=True)


def text_pdf(path, lines, title="doc"):
    """A normal text-layer PDF, with the name in the /Info metadata too -
    which is how a Word or Google Docs export actually leaks it."""
    content = "BT /F1 12 Tf\n"
    y = 720
    for ln in lines:
        esc = ln.replace("\\", r"\\").replace("(", r"\(").replace(")", r"\)")
        content += f"1 0 0 1 72 {y} Tm ({esc}) Tj\n"
        y -= 24
    content += "ET\n"
    cb = content.encode()

    objs = [
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Count 1/Kids[3 0 R]>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]"
        b"/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
        b"<</Length %d>>\nstream\n" % len(cb) + cb + b"\nendstream",
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
        # Metadata the tool must strip.
        ("<</Title(%s)/Author(Jane Doe)/Creator(Microsoft Word)"
         "/Producer(Acme PDF 1.0)>>" % title).encode(),
    ]
    buf = bytearray(b"%PDF-1.4\n")
    offs = []
    for i, o in enumerate(objs, 1):
        offs.append(len(buf))
        buf += b"%d 0 obj\n" % i + o + b"\nendobj\n"
    xref = len(buf)
    buf += b"xref\n0 %d\n" % (len(objs) + 1) + b"0000000000 65535 f \n"
    for o in offs:
        buf += b"%010d 00000 n \n" % o
    buf += (b"trailer\n<</Size %d/Root 1 0 R/Info 6 0 R>>\nstartxref\n%d\n%%%%EOF\n"
            % (len(objs) + 1, xref))
    path.write_bytes(buf)
    return path


def affiliation_pdf(path):
    """An author block in the shape academic papers actually use: the name
    followed by a superscript affiliation marker set flush against it, and a
    correspondence address that cannot be derived from the name."""
    parts = [
        (72, 720, 12, "Authors and Affiliations"),
        (72, 696, 11, "Jane Doe"),
        # Superscript: smaller font, raised baseline, flush against the name.
        (118, 701, 7, "1"),
        (124, 696, 11, "\\267 Heidi Reichert"),
        (72, 660, 10, "btaghiz@ncsu.edu"),
        (72, 640, 10, "Doe J, Reichert H (2025) A paper about things."),
    ]
    content = "".join(
        f"BT /F1 {sz} Tf 1 0 0 1 {x} {y} Tm ({t}) Tj ET\n" for x, y, sz, t in parts
    )
    cb = content.encode()
    objs = [
        b"<</Type/Catalog/Pages 2 0 R>>",
        b"<</Type/Pages/Count 1/Kids[3 0 R]>>",
        b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 612 792]"
        b"/Resources<</Font<</F1 5 0 R>>>>/Contents 4 0 R>>",
        b"<</Length %d>>\nstream\n" % len(cb) + cb + b"\nendstream",
        b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>",
        b"<</Title(affil)/Author(Jane Doe)>>",
    ]
    buf = bytearray(b"%PDF-1.4\n"); offs = []
    for i, o in enumerate(objs, 1):
        offs.append(len(buf)); buf += b"%d 0 obj\n" % i + o + b"\nendobj\n"
    xref = len(buf)
    buf += b"xref\n0 %d\n" % (len(objs) + 1) + b"0000000000 65535 f \n"
    for o in offs:
        buf += b"%010d 00000 n \n" % o
    buf += (b"trailer\n<</Size %d/Root 1 0 R/Info 6 0 R>>\nstartxref\n%d\n%%%%EOF\n"
            % (len(objs) + 1, xref))
    path.write_bytes(buf)
    return path


def image_pdf(path, pages_text):
    """Pages that are pure pixels - the stitched-PNG case, where there is no
    text layer to search and the name exists only as rendered glyphs."""
    imgs = []
    for lines in pages_text:
        im = Image.new("RGB", (850, 1100), "white")
        d = ImageDraw.Draw(im)
        y = 80
        for ln in lines:
            d.text((70, y), ln, fill="black")
            y += 30
        imgs.append(im)
    imgs[0].save(path, "PDF", save_all=True, append_images=imgs[1:], resolution=100)
    return path


if __name__ == "__main__":
    text_pdf(OUT / "homework.pdf", [
        "CSC 116 - Lab 3 Part 1",
        "Name: Jane Doe",
        "Unity ID: jdoe2    Email: jdoe2@ncsu.edu",
        "",
        "Q1: A while loop repeats until its condition is false.",
        "Q2: Jane has 5 apples and gives 2 away. How many remain?",
        "Q3: Submitted by Doe, Jane on March 3.",
    ], title="Lab3 - Jane Doe")

    text_pdf(OUT / "chatlog.pdf", [
        "Gemini",
        "Jane Doe",
        "",
        "You: can you help me with my CSC116 lab?",
        "Gemini: Of course, Jane! What is the assignment?",
        "You: i need to write a while loop",
        "Gemini: Here is an example for you, Jane Doe:",
        "    while count < 10:",
        "        count += 1",
    ], title="Gemini - Jane Doe")

    # Reproduces two failure modes seen on a real document: a name at the start
    # of a wrapped line (producers often omit the EOL flag, which used to glue
    # it to the previous word and lose the match entirely), and the name as a
    # substring of a longer word, which must be left alone but reported.
    text_pdf(OUT / "wrapped.pdf", [
        "CSC 116 - Lab 3",
        "This assignment was submitted by",
        "Jane Doe on March 3rd of this year.",
        "See the Janedoexyz compound which is a different word.",
        "Contact provision.sh",
        "Jane again on the next line.",
    ], title="wrapped")

    affiliation_pdf(OUT / "affil.pdf")

    image_pdf(OUT / "scanned.pdf", [
        ["CSC 116 Worksheet", "Name: Jane Doe", "", "1) x = 5", "2) y = 10"],
        ["Page 2", "Jane Doe - continued", "3) print(x + y)"],
    ])

    for f in sorted(OUT.iterdir()):
        print(f"{f.name:16} {f.stat().st_size:>8,} bytes")
