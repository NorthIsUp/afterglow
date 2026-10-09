// The mirror's stream: the page's copy of the grid, its paint, and the
// /stream connection. A classic script after the page's inline one, so it
// shares that script's top-level names (`meta`, `ctx`, `status`, …).

// ── MODEL ────────────────────────────────────────────────────────────
//
// Records are applied to `have` as they arrive and painted once per
// animation frame, so a page that falls behind paints the newest state
// rather than replaying every frame it missed, and nothing queues: a record
// costs one store per cell, the canvas work happens at most 60 times a
// second whatever the stream does.
//
// `flat` is the grid as one pixel per cell, kept current for every cell.
// While no cell needs a glyph blit — doom, blocks, every picture saver — a
// paint is one putImageData and one scaled drawImage of the changed box
// instead of a fillRect per cell; 167k fillRects a frame is what used to
// take the tab down on doom.
const NEVER = 0xffffffff,
  PACKED = 0x80000000;
let have = new Uint32Array(0),
  dirty = new Uint32Array(0),
  marked = new Uint8Array(0),
  nDirty = 0,
  glyphCells = 0,
  flat = null,
  flatPx = null,
  flatCtx = null,
  palPx = null,
  groundPx = 0,
  queued = false,
  rateAt = 0,
  rateBytes = 0,
  rateFrames = 0,
  rate = "",
  palHex = [];

const px = (c) => 0xff000000 | ((c & 255) << 16) | (c & 0xff00) | ((c >> 16) & 255);

function resetModel(m) {
  const n = m.cols * m.rows;
  have = new Uint32Array(n).fill(NEVER);
  dirty = new Uint32Array(n);
  marked = new Uint8Array(n);
  nDirty = glyphCells = 0;
  flatCtx = new OffscreenCanvas(m.cols, m.rows).getContext("2d");
  flat = flatCtx.createImageData(m.cols, m.rows);
  flatPx = new Uint32Array(flat.data.buffer);
  palPx = Uint32Array.from(m.palette, px);
  groundPx = px(m.ground);
  palHex = m.palette.map(hex);
}

// NEVER's glyph is past the table, so a cell not yet received is flat.
const isGlyph = (cell) => {
  const g = cell & 0xffff;
  return g < solid.length && !solid[g] && !blank[g];
};

function set(i, cell) {
  const old = have[i];
  if (old === cell) return;
  glyphCells += isGlyph(cell) - isGlyph(old);
  have[i] = cell;
  flatPx[i] = solid[cell & 0xffff] ? palPx[cell >>> 16] : groundPx;
  cells++;
  if (!marked[i]) {
    marked[i] = 1;
    dirty[nDirty++] = i;
  }
}

// `base` is the record's offset inside the shared receive buffer.
function applySparse(view, base) {
  const n = view.getUint32(base, true);
  for (let k = 0; k < n; k++) {
    const o = base + 4 + k * 8;
    set(view.getUint32(o, true), view.getUint32(o + 4, true));
  }
}

function applyPacked(values, idx) {
  const ix =
    values.length > 256 ? new Uint16Array(idx.buffer, idx.byteOffset, idx.length >> 1) : idx;
  for (let i = 0; i < ix.length; i++) {
    const v = values[ix[i]];
    if (v !== NEVER) set(i, v);
  }
}

// Native inflate: no decoder shipped in the page, and none of it in JS.
async function inflate(bytes) {
  const z = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate-raw"));
  return new Uint8Array(await new Response(z).arrayBuffer());
}

function queuePaint() {
  if (queued || !nDirty) return;
  queued = true;
  requestAnimationFrame(paint);
}

function paint() {
  queued = false;
  if (!meta || !nDirty) return;
  const { cols, rows, cell_w: cw, cell_h: ch } = meta;
  if (unpainted) {
    unpainted = false;
    ctx.fillStyle = hex(meta.ground);
    ctx.fillRect(0, 0, canvas.width, canvas.height);
  }
  const n = nDirty;
  if (glyphCells === 0) {
    let x0 = cols,
      y0 = rows,
      x1 = 0,
      y1 = 0;
    for (let k = 0; k < n; k++) {
      const i = dirty[k];
      marked[i] = 0;
      const x = i % cols,
        y = (i / cols) | 0;
      if (x < x0) x0 = x;
      if (x > x1) x1 = x;
      if (y < y0) y0 = y;
      if (y > y1) y1 = y;
    }
    const w = x1 - x0 + 1,
      h = y1 - y0 + 1;
    flatCtx.putImageData(flat, 0, 0, x0, y0, w, h);
    ctx.drawImage(flatCtx.canvas, x0, y0, w, h, x0 * cw, y0 * ch, w * cw, h * ch);
  } else {
    const { glyph_w: gw, glyph_h: gh } = meta;
    const groundHex = hex(meta.ground);
    for (let k = 0; k < n; k++) {
      const i = dirty[k];
      marked[i] = 0;
      const cell = have[i];
      const glyph = cell & 0xffff,
        colour = cell >>> 16;
      const x = (i % cols) * cw,
        y = ((i / cols) | 0) * ch;
      if (solid[glyph]) {
        ctx.fillStyle = palHex[colour];
        ctx.fillRect(x, y, cw, ch);
        continue;
      }
      ctx.fillStyle = groundHex;
      ctx.fillRect(x, y, cw, ch);
      if (!blank[glyph]) ctx.drawImage(atlasFor(colour), glyph * gw, 0, gw, gh, x, y, cw, ch);
    }
  }
  nDirty = 0;
  const now = performance.now();
  if (now - rateAt >= 1000) {
    rate =
      `${String(Math.round((rateFrames * 1000) / (now - rateAt))).padStart(2, "0")} fps · ` +
      `${String(Math.round(rateBytes / (now - rateAt))).padStart(5, "0")} KB/s`;
    rateAt = now;
    rateBytes = rateFrames = 0;
  }
  // A layout-invalidating innerHTML write, so once per paint. Zero-padded
  // to the widest value each field can take — cols*rows — so the line keeps
  // a fixed width. Unpadded, every frame reflows the text and the whole
  // status row twitches sideways 30 times a second, which reads as the page
  // being broken rather than as the numbers changing.
  status.innerHTML =
    `<b>${meta.saver}</b> · ${cols}x${rows} cells · ` +
    `${pad(n)} changed · ${pad(Math.round(cells / Math.max(frames, 1)))} avg · ${rate}`;
}

// ── CONNECTION ───────────────────────────────────────────────────────
//
// One session at a time: a /meta (or one handed over by /select) and the
// /stream for its epoch. Starting a session, or a click, ends the one before
// it — its controller is aborted and its number is stale, so whatever it
// throws on the way out is ignored rather than reported.
//
// A stream ending is routine: a click, a rotation, a settings rebuild, a
// tab culled in the background. WebKit reports a body cut mid-read as
// "Load failed" and Chrome as a network error; none of that is a fault, so
// the page reconnects at once and says nothing. Only a run of sessions
// that never drew a frame, over a few seconds, puts up a banner.
let sid = 0;
let ctl = new AbortController();
// The ground repaints with the first record of a new epoch, not when its
// /meta arrives, so a switch shows the old frame until the new one is
// ready instead of a blank.
let unpainted = false;
// Consecutive sessions that ended before drawing anything, and since when.
let fails = 0,
  failSince = 0;

function closeStream() {
  sid++;
  ctl.abort();
}

async function session(m) {
  closeStream();
  const mine = sid;
  const c = (ctl = new AbortController());
  const st = { drew: false };
  try {
    if (!m) {
      const res = await fetch("/meta", { cache: "no-store", signal: c.signal });
      if (!res.ok) throw new Error(res.status === 503 ? "no display yet" : "/meta " + res.status);
      m = await res.json();
    }
    if (mine !== sid) return;
    applyMeta(m);
    // Hand back the epoch /meta was built for, so a stream for a saver
    // that has since been replaced is refused (409) rather than drawn
    // against the wrong geometry, palette and glyph table.
    const stream = await fetch("/stream?epoch=" + m.epoch, {
      cache: "no-store",
      signal: c.signal,
    });
    if (stream.status === 409) {
      st.drew = true; // a race lost, not a fault: straight back round
      return;
    }
    if (!stream.ok) throw new Error("/stream " + stream.status);
    await read(stream, mine, st);
  } catch (e) {
    if (mine !== sid) return;
    if (!st.drew && !fails++) failSince = performance.now();
    // A run of empty sessions over a few seconds is the pod gone, or a
    // proxy in the way; one or two is a switch landing mid-request.
    if (fails >= 3 && performance.now() - failSince > 3000)
      status.textContent = `${e.message || e.name} — retrying`;
  } finally {
    if (mine === sid) {
      if (st.drew) fails = 0;
      // Straight back for a routine end; backing off only once failing.
      setTimeout(() => mine === sid && session(null), fails ? Math.min(2000, 250 * fails) : 0);
    }
  }
}

// Draw the stream until it ends, noting in `st` once it drew a record.
async function read(stream, mine, st) {
  const reader = stream.body.getReader();
  // Records straddle chunk boundaries, so hold a buffer and consume only
  // whole ones. Grow-only, with a read offset: the obvious
  // allocate-and-copy-everything per chunk is O(n^2) in chunks per frame,
  // and on `blocks` (37k changed cells a frame, 4.4 MB/s) that measured ~3
  // MB/frame of garbage and showed up as GC spikes. Here a chunk is one
  // `set` into spare room, and consumed bytes are reclaimed by resetting the
  // offset — a compaction only happens when a record really did straddle.
  let buf = new Uint8Array(1 << 16);
  let view = new DataView(buf.buffer);
  let len = 0, // bytes written
    off = 0; // bytes already drawn
  for (;;) {
    const { value, done } = await reader.read();
    // The server ends the stream when the epoch moves; `session` reconnects.
    if (done || mine !== sid) return;
    if (len + value.length > buf.length) {
      // Drop what has been drawn, and only then consider growing. A full
      // keyframe on `blocks` is ~1 MB, so the buffer does settle larger than
      // its initial size — it just stops churning once it has.
      buf.copyWithin(0, off, len);
      len -= off;
      off = 0;
      if (len + value.length > buf.length) {
        let cap = buf.length;
        while (cap < len + value.length) cap *= 2;
        const grown = new Uint8Array(cap);
        grown.set(buf.subarray(0, len));
        buf = grown;
        view = new DataView(buf.buffer);
      }
    }
    buf.set(value, len);
    len += value.length;
    rateBytes += value.length;
    for (;;) {
      if (len - off < 4) break;
      const head = view.getUint32(off, true);
      const need = 4 + (head & PACKED ? head & ~PACKED : head * 8);
      if (len - off < need) break;
      if (head & PACKED) {
        const t = view.getUint32(off + 4, true);
        const values = new Uint32Array(t);
        for (let k = 0; k < t; k++) values[k] = view.getUint32(off + 8 + k * 4, true);
        // Awaited in place: records are deltas and must apply in order, and
        // not reading meanwhile is what pushes back on the server.
        const idx = await inflate(buf.subarray(off + 8 + t * 4, off + need));
        if (mine !== sid) return;
        applyPacked(values, idx);
      } else applySparse(view, off);
      frames++;
      rateFrames++;
      off += need;
      st.drew = true;
    }
    queuePaint();
    // The common case: the chunk ended on a record boundary, so the whole
    // buffer is free again without moving a byte.
    if (off === len) off = len = 0;
  }
}

session(null);
