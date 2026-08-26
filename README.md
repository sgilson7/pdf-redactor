# PDF Redactor

Remove a student's name and all metadata from a PDF, in the browser, with a
review step and a verification pass.

### ▶ [Open the tool](https://sgilson7.github.io/pdf-redactor/)

Nothing is uploaded. There is no server, no account, and no network request
after the page loads — the PDF is opened, redacted, and saved entirely inside
your browser tab. That is not a policy promise; it is a property of how the tool
is built, and you can confirm it by watching the Network tab while you use it.

---

## How it works

You pick a PDF and type a name. The tool searches every page for that name and
its variants, pre-marks the confident hits, and shows you each one in context.
You approve, reject, or draw your own boxes, then export.

The output is built by **rasterising every page and constructing a new PDF from
scratch**. Redaction boxes are painted into the page bitmap *before* it is
encoded, so the covered pixels are not hidden behind an object — they do not
exist in the output file. There is no black rectangle to delete, no object to
recover, and no earlier revision left in the file.

Rebuilding from scratch also means metadata is eliminated by construction rather
than by deleting keys one at a time. The output contains only what the writer
chose to emit, which is why none of the following survive:

| | |
|---|---|
| `/Info` dictionary | author, title, producer, creation and modification dates |
| XMP metadata | including per-page and per-image streams |
| `/PieceInfo` | private application data left by Word and Illustrator |
| Annotations | comments carry author names in `/T` |
| Form fields | `/V` values, often still holding what was typed |
| Embedded files | attachments and `/Names` trees |
| JavaScript | `/OpenAction`, `/JS` |
| Optional content | hidden layers |
| Page thumbnails | a thumbnail can show an unredacted page |
| Prior revisions | incremental-update history — the most common real leak |
| Trailer `/ID` | a fingerprinting vector with no use here |

### Keeping the output searchable

Rasterising normally costs you the text layer. Instead, the original text is
re-attached as **invisible text** — the same technique OCR'd PDFs use — after
every span touching a redaction box has been dropped. The redacted name cannot
appear in that layer because it was never written into it. So `Ctrl-F` and
`grep` still work across an anonymised corpus.

Content streams are deliberately left **uncompressed**. It costs a few KB and
buys you the ability to check the tool's work yourself:

```sh
strings redacted.pdf | grep -i "jane"     # should print nothing
```

## What it finds

Matching runs on a joined, normalised version of each page rather than on raw
text fragments, because PDF producers split text arbitrarily — `Jane Doe`
routinely arrives as `["Ja", "ne D", "oe"]`, which per-fragment matching misses
entirely. Normalisation folds case, strips diacritics (`José` ≡ `Jose`), expands
ligatures, removes zero-width characters, and rejoins words hyphenated across a
line break (`Jo-\nhnson`).

Matching requires **whole words**: redacting `Docker` leaves `Dockerfile`
alone, and a student named Kim is not redacted out of `Kimberly`. Substring
occurrences are counted and reported at export so the choice is visible rather
than silent.

Hits are tiered, and **only high-confidence hits are pre-checked**:

| Tier | Pre-checked | Examples for *Jane Doe* |
|---|---|---|
| **High** | yes | `Jane Doe`, `Doe, Jane`, `JaneDoe`, `jdoe`, `jdoe2`, `jane.doe@ncsu.edu` |
| **Medium** | no | `Jane`, `Doe` on their own |
| **Low** | no | `J. Doe`, `JD`, and near-misses like `Jayne` |

Name tokens that double as ordinary words are demoted automatically. A student
named Will, May, or Song would otherwise have their entire submission blacked
out by the bare first-name variant.

## What it does not do

Being clear about this matters more than the feature list.

- **Pages with no text layer cannot be searched.** Scans and PDFs assembled from
  images have no text to find. The tool detects this, marks those pages, and
  tells you plainly that only your manual boxes apply there. It does not OCR.
- **Names inside images are pixels.** A screenshot showing `C:\Users\jdoe\` will
  not be found by text search. Draw a box.
- **Review is not optional.** Medium and Low hits exist because automatic
  matching cannot tell a student named Jane from a word problem about Jane.
- **Long documents get large.** Rasterising a 125-page book at 200 DPI produces
  roughly 70 MB. The toolbar estimates the size once a document is open; 150 DPI
  is about half. Student submissions of a few pages are unaffected.
- **Text rotated inside the page** — a diagonal watermark, a vertical axis
  label — may get a box that does not sit squarely over it. Whole-page rotation
  is handled correctly. A misplaced box is visible during review and can be
  deleted and redrawn.

## A note on Gemini chatlogs

Chat transcripts differ from homework in a way that matters for review. The
model addresses the student by first name throughout — *"Of course, Jane!"* — so
the bare first name is genuinely identifying there, and the Medium
**first name only** hit is usually worth checking. On a worksheet the same hit
is usually worth leaving alone, because *"Jane has 5 apples"* is the question.

Chrome's print-to-PDF also renders the account display name and the avatar
initial from the Gemini UI. The display name is normally real text and gets
found; the avatar circle may be pixels, and needs a manual box.

## Verification

Every export re-opens the finished document and checks it, and the report
distinguishes two things that are easy to conflate:

- **A term you approved is still present** → a redaction silently failed. This
  is a defect, and the download is blocked.
- **A term you declined is still present** → your informed choice. Reported, not
  blocked. Redacting *"Jane has 5 apples"* would destroy the assignment.

## Building it

Requires Rust and the wasm target. No node, npm, or bundler.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version "$(awk '/^name = "wasm-bindgen"$/{f=1} f&&/^version = /{gsub(/"/,"");print $3;exit}' Cargo.lock)"

make test           # engine test suite (native, no browser)
make test-ui-setup  # one-time: headless Chromium for browser tests
make test-ui        # drive the real UI, screenshot it, verify exports
make serve          # build and open at localhost:8080
make deploy         # push; Actions builds, tests, and publishes to Pages
```

## Layout

```
crates/core/    the redaction engine — no browser dependencies, so all of it
                runs under `cargo test`
  normalize.rs  unicode folding with an index map back to the source
  variants.rs   tiered variant generation
  matching.rs   fragment joining, fuzzy matching, box derivation
  redact.rs     filtering the text layer before it is written
  pdfwrite.rs   the PDF writer — every byte of the output is chosen here
  verify.rs     re-reads the finished file and reports what is in it
crates/wasm/    a thin wasm-bindgen bridge with no logic of its own
web/            the review UI; pdf.js is vendored, not loaded from a CDN
```

Reading is done by [pdf.js](https://mozilla.github.io/pdf.js/), which is the
engine Firefox ships and is hardened against exactly the malformed output that
browser print-to-PDF produces. Writing is hand-rolled rather than delegated to a
PDF library, so that the claim "the output contains only what we wrote" is
something you can check by reading one file.

## Licence

MIT.
