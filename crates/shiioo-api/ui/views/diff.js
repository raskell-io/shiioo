// The run's branch as a reviewer reads it: a file list with counts, then
// each file's hunks with line numbers. Parsed from `git diff --stat -p`.

import { el, esc } from "../core.js";

const MAX_LINES = 600; // per file, before "show the rest"

/** Files of a unified diff: `{ name, adds, dels, hunks: [{ head, lines: [{ t, text, old, new }] }] }`. */
export function parseDiff(text) {
  const files = [];
  let file = null;
  let hunk = null;
  let oldN = 0;
  let newN = 0;
  for (const line of text.split("\n")) {
    if (line.startsWith("diff --git ")) {
      const m = line.match(/ b\/(.*)$/);
      file = { name: m ? m[1] : line.slice(11), adds: 0, dels: 0, hunks: [], binary: false };
      files.push(file);
      hunk = null;
      continue;
    }
    if (!file) continue;
    if (line.startsWith("@@")) {
      const m = line.match(/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/);
      oldN = m ? Number(m[1]) : 0;
      newN = m ? Number(m[2]) : 0;
      hunk = { head: m ? m[3].trim() : line, lines: [] };
      file.hunks.push(hunk);
      continue;
    }
    if (!hunk) {
      if (line.startsWith("Binary files")) file.binary = true;
      if (line.startsWith("rename to ")) file.name = line.slice(10);
      continue;
    }
    if (line.startsWith("+")) { hunk.lines.push({ t: "a", text: line.slice(1), new: newN++ }); file.adds++; }
    else if (line.startsWith("-")) { hunk.lines.push({ t: "d", text: line.slice(1), old: oldN++ }); file.dels++; }
    else if (line.startsWith(" ")) { hunk.lines.push({ t: "ctx", text: line.slice(1), old: oldN++, new: newN++ }); }
  }
  return files;
}

const counts = (f) => `<span class="st"><span class="p">+${f.adds}</span> <span class="m">−${f.dels}</span></span>`;

/** Renders the file list and every file into `box`. */
export function renderDiff(box, text, { wrap = false } = {}) {
  const files = parseDiff(text || "");
  box.innerHTML = "";
  if (!files.length) {
    box.appendChild(el(`<div class="aside-empty">No changes on the branch yet.</div>`));
    return;
  }
  const adds = files.reduce((n, f) => n + f.adds, 0);
  const dels = files.reduce((n, f) => n + f.dels, 0);
  const list = el(`<div class="files"><div class="files-h">${files.length} ${files.length === 1 ? "file" : "files"}
    ${counts({ adds, dels })}</div></div>`);
  box.appendChild(list);
  files.forEach((f, i) => {
    const b = el(`<button class="file" type="button" title="${esc(f.name)}"><span class="n">‎${esc(f.name)}</span>${counts(f)}</button>`);
    b.addEventListener("click", () => box.querySelector(`[data-file="${i}"]`)?.scrollIntoView({ behavior: "smooth", block: "start" }));
    list.appendChild(b);
  });
  files.forEach((f, i) => {
    const view = el(`<div class="diffview" data-file="${i}"><div class="fh"><span>${esc(f.name)}</span>${counts(f)}</div></div>`);
    const body = el(`<div class="idiff${wrap ? " wrap" : ""}"></div>`);
    if (f.binary) body.appendChild(el(`<div class="hunk">Binary file</div>`));
    let shown = 0;
    let html = "";
    const rest = [];
    for (const h of f.hunks) {
      const chunk = [`<div class="hunk">${esc(h.head || "…")}</div>`];
      for (const l of h.lines) {
        const n = l.t === "d" ? l.old : l.new;
        const sign = l.t === "a" ? "+" : l.t === "d" ? "−" : " ";
        chunk.push(`<div class="${l.t}"><span class="ln">${n}</span><span class="sign">${sign}</span>${esc(l.text)}</div>`);
      }
      if (shown < MAX_LINES) html += chunk.join(""); else rest.push(chunk.join(""));
      shown += h.lines.length;
    }
    body.innerHTML = html;
    if (rest.length) {
      const more = el(`<div class="hunk" role="button" tabindex="0" style="cursor:pointer">Show ${shown - MAX_LINES > 0 ? "the rest" : "more"} of this file</div>`);
      const open = () => { more.remove(); body.insertAdjacentHTML("beforeend", rest.join("")); };
      more.addEventListener("click", open);
      more.addEventListener("keydown", (e) => { if (e.key === "Enter") open(); });
      body.appendChild(more);
    }
    view.appendChild(body);
    box.appendChild(view);
  });
}
