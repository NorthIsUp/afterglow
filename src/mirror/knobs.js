// The mirror's per-saver config bar. A classic script after the page's
// inline one, so it shares that script's top-level names (`meta`, `el`, …).
//
// Whatever knobs the saver's constructor reads, as /config reports them —
// the page knows no saver's knobs. The server validates against the
// constructor's own range, and rebuilds the saver when it reads that key,
// which ends this stream like any switch.
const knobs = document.getElementById("knobs");
let cfgFor = null; // the epoch the knobs were fetched for

async function loadConfig(force) {
  if (!force && cfgFor === meta.epoch) return;
  cfgFor = meta.epoch;
  const name = meta.saver;
  document.getElementById("cfgname").textContent = label(name);
  const res = await fetch("/config?saver=" + encodeURIComponent(name), {
    cache: "no-store",
  });
  if (!res.ok || name !== meta.saver) return;
  showKnobs(name, await res.json());
}

// What the rows were built from. The reconnect after a write fetches the
// same list the write already returned; rebuilding the rows for it would
// throw away whatever the viewer has started typing in the next box.
let knobsShown = "";
// Rows with a step still waiting to post. Rebuilding then would put the
// last answer's value back in the box mid-burst and drop the waiting step.
const stepping = new Set();

function showKnobs(name, list) {
  const json = name + JSON.stringify(list);
  if (json === knobsShown || stepping.size) return;
  knobsShown = json;
  const focused = document.activeElement.closest(".knob")?.dataset.key;
  knobs.replaceChildren(
    ...list.map((k) => knobRow(name, k)),
    ...(list.length ? [] : [el("span", { textContent: "no settings" })]),
  );
  // Arrow keys keep stepping the same box across the rebuild a step causes.
  knobs.querySelector(`[data-key="${focused}"] input`)?.focus();
}

function knobRow(name, k) {
  const row = el("label", {
    className: "knob" + (k.overridden ? " set" : ""),
    title: [k.key, k.help].filter(Boolean).join(" — "),
  });
  row.dataset.key = k.key;
  const err = el("span", { className: "err" });
  let input;
  if (k.kind === "bool") {
    input = el("input", { type: "checkbox", checked: k.value === 1 });
    row.append(input, el("span", { textContent: k.label }));
  } else {
    // A handful of steps (a skill level) reads better as a slider.
    const slider = k.kind === "num" && k.hi - k.lo <= 10;
    input =
      k.kind === "num"
        ? el("input", {
            type: slider ? "range" : "number",
            min: k.lo,
            max: k.hi,
            step: 1,
            value: k.value,
          })
        : el("input", { type: "text", value: k.value, maxLength: 64 });
    row.append(el("span", { textContent: k.label }), input);
    if (slider) {
      const shown = el("span", { textContent: k.value });
      input.oninput = () => (shown.textContent = input.value);
      row.append(shown);
    }
  }
  let wait;
  const commit = async () => {
    clearTimeout(wait);
    // A held spinner arrow keeps stepping, and the post would replace it.
    if (input.matches(":active")) return void (wait = setTimeout(commit, 250));
    stepping.delete(row);
    const v = k.kind === "bool" ? (input.checked ? 1 : 0) : input.value;
    const r = await send(name, "POST", k.key, v);
    if (r.error) {
      err.textContent = r.error.replace(k.key + ": ", "");
      // Back to what the saver is really using: a refused value changes
      // nothing on the panel, so it must not stay in the box either.
      if (k.kind === "bool") input.checked = k.value === 1;
      else input.value = k.value;
    }
  };
  // Spinner clicks and arrow keys fire `change` on every step, and each post
  // rebuilds the saver: steps (input events without an inputType — typing
  // has one) post once they pause. A slider still posts on release.
  input.addEventListener("input", (e) => {
    if (input.type !== "number" || e.inputType) return;
    clearTimeout(wait);
    stepping.add(row);
    wait = setTimeout(commit, 250);
  });
  input.onchange = () => stepping.has(row) || commit();
  if (k.overridden) {
    const reset = el("button", {
      textContent: "↺",
      title: "back to " + k.default,
    });
    reset.onclick = (e) => {
      e.preventDefault();
      send(name, "DELETE", k.key);
    };
    row.append(reset);
  }
  row.append(err);
  return row;
}

async function send(name, method, key, value) {
  let q =
    "/config?saver=" +
    encodeURIComponent(name) +
    "&key=" +
    encodeURIComponent(key);
  if (value !== undefined) q += "&value=" + encodeURIComponent(value);
  const res = await fetch(q, { method, cache: "no-store" });
  const body = await res.json();
  if (!res.ok) return body;
  // The new list now, before the rebuild's reconnect re-fetches it: a knob
  // that shows or hides others (the tour's switch) should look instant.
  if (name === meta.saver) showKnobs(name, body.knobs);
  // A rebuild is a switch to the same saver: the answer carries its /meta,
  // once it is on the panel, and the stream reopens on that.
  if (body.rebuilt) session(body.meta);
  return {};
}
