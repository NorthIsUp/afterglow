// Render ascii.rest pieces with upstream's own code, for the port's golden test.
//
//   git clone https://github.com/bas3line/ascii /tmp/ascii
//   bun tools/ascii-rest-golden.ts /tmp/ascii /tmp/golden night-coast plasma ...
//   ASCII_REST_GOLDEN=/tmp/golden cargo test --release ascii_rest -- --ignored --nocapture
//
// Every tick from 0 is rendered in order, as `Play` does, so stateful pieces
// (pendulum, reaction-diffusion) see the same sequence of `t`. Only TICKS are
// written: per tick, the text rows, then for a coloured piece one row of two
// hex digits per cell.

const [root, out, ...names] = process.argv.slice(2);
const TICKS = (fps: number) => [0, 1, fps * 3 + 7, fps * 20 + 3];

for (const name of names) {
  const mod = await import(`${root}/src/pieces/${name}.ts`);
  const { cols, rows, fps, palette } = mod.meta;
  const frame = mod.default();
  const want = new Set(TICKS(fps));
  const last = Math.max(...want);
  const color = palette ? new Uint8Array(cols * rows) : undefined;
  let txt = `${cols} ${rows} ${fps}\n`;
  for (let tick = 0; tick <= last; tick++) {
    const s: string = frame(tick / fps, { color });
    if (!want.has(tick)) continue;
    txt += `@${tick}\n${s}\n`;
    if (color) {
      for (let r = 0; r < rows; r++) {
        txt +=
          Array.from(color.subarray(r * cols, (r + 1) * cols), (v) =>
            v.toString(16).padStart(2, "0"),
          ).join("") + "\n";
      }
    }
  }
  await Bun.write(`${out}/${name}.golden`, txt);
  console.log(`${name}: ${want.size} frames`);
}
