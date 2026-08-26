// PDF Redactor — browser front end.
//
// Division of labour: pdf.js parses and rasterises, Rust/wasm decides what to
// redact and writes the output. Nothing here makes a redaction decision on its
// own, so the logic that has to be correct stays under `cargo test`.
//
// One coordinate space is used for everything the user manipulates: pdf.js
// viewport at scale 1 (origin top-left, y down, units of PDF points). It
// already accounts for page rotation, and being independent of both preview
// zoom and export DPI is what keeps boxes from drifting.

import * as pdfjs from './vendor/pdfjs/pdf.mjs';
import init, { find_matches, Builder, verifyOutput, mergeBoxes } from './pkg/redactor_wasm.js';

pdfjs.GlobalWorkerOptions.workerSrc = './vendor/pdfjs/pdf.worker.mjs';
const PDFJS_ASSETS = {
  cMapUrl: './vendor/pdfjs/cmaps/',
  cMapPacked: true,
  standardFontDataUrl: './vendor/pdfjs/standard_fonts/',
};

const $ = (id) => document.getElementById(id);
const state = {
  doc: null,
  pages: [],     // { items, w, h, noText }
  hits: [],      // { id, page, tier, label, matched, context, boxes, on }
  manual: [],    // { id, page, box }
  cur: 0,
  scale: 1,
  terms: { approved: [], declined: [] },
};

// ---------------------------------------------------------------- loading

async function boot() {
  await init();
  wireDropZone();
  $('scan').onclick = scan;
  $('export').onclick = doExport;
  $('mclose').onclick = () => ($('modal').hidden = true);
  $('name').addEventListener('keydown', (e) => e.key === 'Enter' && scan());
}

function wireDropZone() {
  const drop = $('drop');
  $('file').onchange = (e) => e.target.files[0] && open(e.target.files[0]);
  for (const ev of ['dragenter', 'dragover']) {
    drop.addEventListener(ev, (e) => { e.preventDefault(); drop.classList.add('over'); });
  }
  for (const ev of ['dragleave', 'drop']) {
    drop.addEventListener(ev, (e) => { e.preventDefault(); drop.classList.remove('over'); });
  }
  drop.addEventListener('drop', (e) => {
    const f = [...e.dataTransfer.files].find((f) => f.type === 'application/pdf');
    if (f) open(f);
  });
}

async function open(file) {
  busy('Reading document…');
  try {
    const data = new Uint8Array(await file.arrayBuffer());
    state.doc = await pdfjs.getDocument({ data, ...PDFJS_ASSETS }).promise;
    state.origName = file.name.replace(/\.pdf$/i, '');
    state.pages = [];
    state.hits = [];
    state.manual = [];

    for (let p = 1; p <= state.doc.numPages; p++) {
      busy(`Reading page ${p} of ${state.doc.numPages}…`);
      const page = await state.doc.getPage(p);
      const vp = page.getViewport({ scale: 1 });
      const items = await readTextItems(page, vp);
      state.pages.push({
        items,
        w: vp.width,
        h: vp.height,
        // A page whose text layer is empty cannot be searched at all. This is
        // the honest failure mode for scans and stitched-image PDFs, and it has
        // to be surfaced rather than quietly passed over.
        noText: items.every((i) => !i.text.trim()),
      });
    }

    $('drop').hidden = true;
    $('app').hidden = false;
    await buildRail();
    await show(0);
    $('name').focus();
  } catch (e) {
    alert(`Could not open that PDF.\n\n${e.message}`);
  } finally {
    idle();
  }
}

/// Convert pdf.js text items into the shape `core` expects.
async function readTextItems(page, vp) {
  const tc = await page.getTextContent();
  const out = [];
  for (const it of tc.items) {
    if (!it.str) continue;
    // Compose the item transform with the viewport's, exactly as pdf.js's own
    // text layer does, so positions line up with what gets rendered.
    const tx = pdfjs.Util.transform(vp.transform, it.transform);
    const h = Math.hypot(tx[2], tx[3]);
    out.push({
      text: it.str,
      x: tx[4],
      // tx[5] is the baseline; `core` wants the top of the line box.
      y: tx[5] - h,
      w: it.width || it.str.length * h * 0.5,
      h,
      eol: !!it.hasEOL,
    });
  }
  return out;
}

// ---------------------------------------------------------------- scanning

function scan() {
  const name = $('name').value.trim();
  if (!name) { $('name').focus(); return; }
  const extras = $('extras').value.split(',').map((s) => s.trim()).filter(Boolean);

  state.hits = [];
  let id = 0;
  for (let p = 0; p < state.pages.length; p++) {
    const found = JSON.parse(find_matches(
      JSON.stringify(state.pages[p].items), name, JSON.stringify(extras),
    ));
    for (const m of found) {
      state.hits.push({
        id: id++, page: p, tier: m.tier, label: m.label,
        matched: m.matched, context: m.context, boxes: m.boxes,
        // Only high-confidence hits are pre-checked. Everything else is found
        // and offered, but requires a deliberate click.
        on: m.tier === 'high',
      });
    }
  }
  renderList();
  renderBoxes();
  $('export').disabled = false;
}

// ---------------------------------------------------------------- rendering

async function buildRail() {
  const rail = $('rail');
  rail.innerHTML = '';
  for (let p = 0; p < state.pages.length; p++) {
    const d = document.createElement('div');
    d.className = 'thumb';
    d.onclick = () => show(p);
    const c = document.createElement('canvas');
    const page = await state.doc.getPage(p + 1);
    const vp = page.getViewport({ scale: 96 / state.pages[p].w });
    c.width = vp.width; c.height = vp.height;
    await page.render({ canvasContext: c.getContext('2d'), viewport: vp }).promise;
    d.appendChild(c);
    const n = document.createElement('span');
    n.className = 'n'; n.textContent = p + 1;
    d.appendChild(n);
    if (state.pages[p].noText) {
      const w = document.createElement('span');
      w.className = 'warn'; w.textContent = '⚠'; w.title = 'No text layer';
      d.appendChild(w);
    }
    rail.appendChild(d);
  }
}

async function show(idx) {
  state.cur = idx;
  [...$('rail').children].forEach((c, i) => c.classList.toggle('sel', i === idx));

  const page = await state.doc.getPage(idx + 1);
  const avail = Math.min(900, window.innerWidth - 520);
  state.scale = Math.max(0.3, avail / state.pages[idx].w);
  const vp = page.getViewport({ scale: state.scale });
  const canvas = $('canvas');
  canvas.width = vp.width; canvas.height = vp.height;
  const ctx = canvas.getContext('2d');
  ctx.fillStyle = '#fff'; ctx.fillRect(0, 0, canvas.width, canvas.height);
  await page.render({ canvasContext: ctx, viewport: vp }).promise;

  const banner = $('banner');
  if (state.pages[idx].noText) {
    banner.hidden = false;
    banner.textContent =
      `⚠ Page ${idx + 1} has no text layer. Automatic detection cannot run here — ` +
      `draw any redactions by hand.`;
  } else {
    banner.hidden = true;
  }
  renderBoxes();
}

/// Boxes live as DOM overlays during review so they stay clickable. They are
/// only painted into pixels at export.
function renderBoxes() {
  const ov = $('overlay');
  ov.innerHTML = '';
  const s = state.scale;

  for (const h of state.hits) {
    if (h.page !== state.cur || !h.on) continue;
    for (const b of h.boxes) ov.appendChild(boxEl(b, s, () => toggle(h.id)));
  }
  for (const m of state.manual) {
    if (m.page !== state.cur) continue;
    ov.appendChild(boxEl(m.box, s, () => {
      state.manual = state.manual.filter((x) => x.id !== m.id);
      renderBoxes(); renderList();
    }));
  }
  attachDraw(ov);
}

function boxEl(b, s, onClick) {
  const d = document.createElement('div');
  d.className = 'rbox';
  d.style.left = `${b.x * s}px`;
  d.style.top = `${b.y * s}px`;
  d.style.width = `${b.w * s}px`;
  d.style.height = `${b.h * s}px`;
  d.title = 'Click to remove';
  d.onclick = (e) => { e.stopPropagation(); onClick(); };
  return d;
}

/// Drag on empty space to add a redaction by hand.
function attachDraw(ov) {
  let start = null, ghost = null;
  ov.onmousedown = (e) => {
    if (e.target !== ov) return;
    const r = ov.getBoundingClientRect();
    start = { x: e.clientX - r.left, y: e.clientY - r.top };
    ghost = document.createElement('div');
    ghost.className = 'rbox pending';
    ov.appendChild(ghost);
  };
  ov.onmousemove = (e) => {
    if (!start) return;
    const r = ov.getBoundingClientRect();
    const x = e.clientX - r.left, y = e.clientY - r.top;
    Object.assign(ghost.style, {
      left: `${Math.min(x, start.x)}px`, top: `${Math.min(y, start.y)}px`,
      width: `${Math.abs(x - start.x)}px`, height: `${Math.abs(y - start.y)}px`,
    });
  };
  ov.onmouseup = (e) => {
    if (!start) return;
    const r = ov.getBoundingClientRect();
    const x = e.clientX - r.left, y = e.clientY - r.top;
    const s = state.scale;
    const box = {
      x: Math.min(x, start.x) / s, y: Math.min(y, start.y) / s,
      w: Math.abs(x - start.x) / s, h: Math.abs(y - start.y) / s,
    };
    start = null; ghost?.remove(); ghost = null;
    if (box.w < 3 || box.h < 3) return;   // ignore stray clicks
    state.manual.push({ id: `m${Date.now()}`, page: state.cur, box });
    renderBoxes(); renderList();
    $('export').disabled = false;
  };
}

// ---------------------------------------------------------------- review list

function toggle(id) {
  const h = state.hits.find((x) => x.id === id);
  h.on = !h.on;
  renderList(); renderBoxes();
}

function renderList() {
  const list = $('list');
  list.innerHTML = '';
  const order = { high: 0, medium: 1, low: 2 };
  const sorted = [...state.hits].sort(
    (a, b) => order[a.tier] - order[b.tier] || a.page - b.page,
  );

  const onCount = state.hits.filter((h) => h.on).length + state.manual.length;
  $('count').textContent = `${onCount} of ${state.hits.length + state.manual.length}`;

  if (!sorted.length && !state.manual.length) {
    list.innerHTML = '<p class="empty">Enter a name and press <strong>Find</strong>.</p>';
    return;
  }

  for (const h of sorted) {
    const el = document.createElement('label');
    el.className = 'hit';
    el.onclick = (e) => { if (e.target.tagName !== 'INPUT') { e.preventDefault(); } };
    const cb = document.createElement('input');
    cb.type = 'checkbox'; cb.checked = h.on;
    cb.onchange = () => { h.on = cb.checked; renderList(); renderBoxes(); };
    const body = document.createElement('div');
    body.innerHTML =
      `<div class="m"></div>` +
      `<div class="meta"><span class="tier ${h.tier}">${h.tier}</span>` +
      `<span>${h.label}</span><span>· page ${h.page + 1}</span></div>` +
      `<div class="ctx"></div>`;
    body.querySelector('.m').textContent = h.matched;
    // Context is shown so a real name can be told apart from a coincidence.
    body.querySelector('.ctx').textContent = h.context;
    body.onclick = () => show(h.page);
    el.append(cb, body);
    list.appendChild(el);
  }

  for (const m of state.manual) {
    const el = document.createElement('div');
    el.className = 'hit';
    el.innerHTML =
      `<input type="checkbox" checked disabled>` +
      `<div><div class="m">Manual redaction</div>` +
      `<div class="meta"><span class="tier manual">manual</span>` +
      `<span>· page ${m.page + 1}</span></div></div>`;
    el.onclick = () => show(m.page);
    list.appendChild(el);
  }
}

// ---------------------------------------------------------------- export

function approvedBoxes(page) {
  const out = [];
  for (const h of state.hits) if (h.page === page && h.on) out.push(...h.boxes);
  for (const m of state.manual) if (m.page === page) out.push(m.box);
  return JSON.parse(mergeBoxes(JSON.stringify(out)));
}

async function doExport() {
  const dpi = +$('dpi').value;
  const wantText = $('textlayer').checked;
  const scale = dpi / 72;
  const builder = new Builder();

  busy('Rendering pages…');
  try {
    for (let p = 0; p < state.pages.length; p++) {
      busy(`Rendering page ${p + 1} of ${state.pages.length}…`);
      const page = await state.doc.getPage(p + 1);
      const vp = page.getViewport({ scale });
      const canvas = document.createElement('canvas');
      canvas.width = Math.round(vp.width);
      canvas.height = Math.round(vp.height);
      const ctx = canvas.getContext('2d');
      // Paint white first: PDF pages have no inherent background, and an
      // unpainted canvas is transparent, which JPEG would render as black.
      ctx.fillStyle = '#fff';
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      await page.render({ canvasContext: ctx, viewport: vp }).promise;

      // Burn the redactions into the bitmap. After this the covered pixels do
      // not exist anywhere in the pipeline — this is the whole guarantee.
      const boxes = approvedBoxes(p);
      ctx.fillStyle = '#000';
      for (const b of boxes) {
        ctx.fillRect(b.x * scale, b.y * scale, b.w * scale, b.h * scale);
      }

      // Hand Rust already-compressed JPEG bytes rather than raw pixels: they go
      // straight into the PDF stream, so nothing large crosses the boundary.
      const blob = await new Promise((r) => canvas.toBlob(r, 'image/jpeg', 0.9));
      const bytes = new Uint8Array(await blob.arrayBuffer());

      builder.addPage(
        bytes, canvas.width, canvas.height, state.pages[p].w, state.pages[p].h,
        JSON.stringify(state.pages[p].items), JSON.stringify(boxes), wantText,
      );
      // Release the bitmap before the next page. Holding them all is what would
      // exhaust memory on a long document.
      canvas.width = canvas.height = 0;
    }

    busy('Verifying output…');
    const pdf = builder.finish();
    const approved = [...new Set(state.hits.filter((h) => h.on).map((h) => h.matched))];
    const declined = [...new Set(state.hits.filter((h) => !h.on).map((h) => h.matched))];
    const report = JSON.parse(verifyOutput(
      pdf, JSON.stringify(approved), JSON.stringify(declined),
      builder.pageCount, builder.redactionCount,
    ));
    showReport(report, pdf);
  } catch (e) {
    alert(`Export failed.\n\n${e.message ?? e}`);
  } finally {
    idle();
  }
}

function showReport(report, pdf) {
  const blocking = report.findings.filter((f) => !('Residual' in f));
  const residual = report.findings.filter((f) => 'Residual' in f);
  const ok = blocking.length === 0;

  const rows = [];
  const row = (cls, ic, text) => rows.push(
    `<div class="chk ${cls}"><span class="ic">${ic}</span><span>${text}</span></div>`,
  );

  row('ok', '✓', `${report.redactions} redaction${report.redactions === 1 ? '' : 's'} ` +
      `applied across ${report.pages} page${report.pages === 1 ? '' : 's'}`);

  if (ok) {
    row('ok', '✓', 'No approved term survives in the output');
    row('ok', '✓', 'No metadata, annotations, or embedded files present');
    row('ok', '✓', 'Single revision — no recoverable earlier version');
  } else {
    for (const f of blocking) {
      if (f.Leak) row('bad', '✕', `Leak: “${f.Leak.term}” still present in ${f.Leak.where_}`);
      else if (f.Structure) row('bad', '✕', `Unexpected object: ${f.Structure}`);
      else if (f.MultipleRevisions !== undefined) {
        row('bad', '✕', `Document contains ${f.MultipleRevisions} revisions`);
      }
    }
  }

  const noText = state.pages.map((p, i) => (p.noText ? i + 1 : 0)).filter(Boolean);
  if (noText.length) {
    row('warn', '⚠', `Page${noText.length > 1 ? 's' : ''} ${noText.join(', ')} had no text ` +
        `layer — only manual redactions applied there`);
  }
  for (const f of residual) {
    row('warn', '⚠', `“${f.Residual.term}” appears ${f.Residual.count}× in the output — ` +
        `you chose not to redact it`);
  }

  row('ok', 'ℹ', `Output is ${(report.bytes / 1024 / 1024).toFixed(2)} MB`);

  $('mtitle').textContent = ok ? 'Verification passed' : 'Verification FAILED';
  $('mbody').innerHTML = rows.join('') + (ok ? '' :
    `<p style="color:var(--danger);margin-top:14px">Download is blocked because a term you ` +
    `approved for redaction is still present. This is a bug in the tool, not a review mistake — ` +
    `please report it.</p>`);

  const a = $('mdownload');
  if (ok) {
    const blob = new Blob([pdf], { type: 'application/pdf' });
    if (a.href.startsWith('blob:')) URL.revokeObjectURL(a.href);
    a.href = URL.createObjectURL(blob);
    // The original filename routinely contains the student's name.
    a.download = 'redacted.pdf';
    a.style.display = '';
  } else {
    a.removeAttribute('href');
    a.style.display = 'none';
  }
  $('modal').hidden = false;
}

// ---------------------------------------------------------------- chrome

function busy(t) { $('busytext').textContent = t; $('busy').hidden = false; }
function idle() { $('busy').hidden = true; }

boot();
