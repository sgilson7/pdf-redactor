// OCR for pages whose text lives in pixels.
//
// The engine's word boxes become ordinary `TextItem`s, so everything
// downstream -- matching, variant tiering, identifier scanning, the review
// list, the redaction boxes -- is the same code that handles a text layer.
// Nothing here knows what a name is.
//
// Assets load on first use, never at startup: they are ~8 MB against a 5.5 MB
// app, and most documents never need them.

const ASSETS = './vendor/tesseract/';
/// OCR wants roughly 300 DPI to read body text reliably. This is deliberately
/// independent of the export DPI -- exporting at 150 should not make detection
/// worse.
const OCR_SCALE = 300 / 72;
/// Below this the engine is guessing; the word is dropped rather than fed to
/// the matcher as though it were read text.
const MIN_CONFIDENCE = 0.30;

let clientPromise = null;

/// Load the engine once and reuse it. `onStage` reports the one-time download.
async function client(onStage) {
  if (!clientPromise) {
    clientPromise = (async () => {
      onStage?.('Preparing OCR engine (one-time download)…');
      const { OCRClient } = await import(`${ASSETS}lib.js`);
      const c = new OCRClient({
        workerURL: `${ASSETS}tesseract-worker.js`,
        corePath: ASSETS,
      });
      await c.loadModel(`${ASSETS}eng.traineddata`);
      return c;
    })().catch((e) => {
      clientPromise = null;   // let a later attempt retry rather than wedge
      throw e;
    });
  }
  return clientPromise;
}

export function isLoaded() {
  return clientPromise !== null;
}

/// OCR one page, returning items in the canonical viewport-at-scale-1 space.
///
/// `pdfPage` is a pdf.js page. `onStage` and `onProgress` drive the busy
/// indicator; OCR of a dense page takes a noticeable moment.
export async function ocrPage(pdfPage, onStage, onProgress) {
  const c = await client(onStage);

  const vp = pdfPage.getViewport({ scale: OCR_SCALE });
  const canvas = document.createElement('canvas');
  canvas.width = Math.round(vp.width);
  canvas.height = Math.round(vp.height);
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  // Paint white first: an unpainted canvas is transparent, which OCR reads as
  // black-on-black and returns nothing for.
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  await pdfPage.render({ canvasContext: ctx, viewport: vp }).promise;

  // Flatten to grey before reading. Tesseract's region classification uses
  // colour, and saturated text gets treated as graphics and skipped outright:
  // on a syntax-highlighted terminal screenshot it read the grey lines at 0.97
  // and silently dropped the green one containing "/Users/jdoe2/...". Removing
  // the colour recovered it, taking the page from 24 words to 28. That matters
  // here because coloured code and terminal output are everywhere in student
  // work and chat transcripts.
  //
  // An inverted second pass was tried too and dropped: tesseract handles
  // light-on-dark natively (0.90 on green-on-#1e1e1e) and inverting returned an
  // identical word list for twice the time.
  await c.loadImage(greyscale(ctx.getImageData(0, 0, canvas.width, canvas.height)));
  const words = await c.getTextBoxes('word', onProgress);
  await c.clearImage();
  canvas.width = canvas.height = 0;   // release before the next page

  const items = [];
  let sum = 0, low = 0;
  for (const w of words) {
    const text = w.text.trim();
    if (!text) continue;
    if (w.confidence < MIN_CONFIDENCE) { low += 1; continue; }
    sum += w.confidence;
    items.push({
      text,
      // Image pixels -> points. Every coordinate the tool stores lives in this
      // one space, which is what keeps boxes correct across zoom and export DPI.
      x: w.rect.left / OCR_SCALE,
      y: w.rect.top / OCR_SCALE,
      w: (w.rect.right - w.rect.left) / OCR_SCALE,
      h: (w.rect.bottom - w.rect.top) / OCR_SCALE,
      // No EOL flags from OCR; the matcher infers line breaks from geometry.
      eol: false,
      confidence: w.confidence,
    });
  }

  return {
    items,
    meanConfidence: items.length ? sum / items.length : 0,
    wordsBelowThreshold: low,
  };
}

/// Flatten to luminance in place. Rec.709 weights, matching how the eye reads
/// contrast, so text that looks legible stays legible to the engine.
function greyscale(image) {
  const d = image.data;
  for (let i = 0; i < d.length; i += 4) {
    const l = 0.2126 * d[i] + 0.7152 * d[i + 1] + 0.0722 * d[i + 2];
    d[i] = d[i + 1] = d[i + 2] = l;
  }
  return image;
}

export const ENGINE = 'tesseract-wasm 0.11.0';
