/** Editable list of URLs: one input with a remove button per entry. */
export type UrlListOptions = {
  placeholder: string;
  removeLabel: string;
  onChange: () => void;
  /** Called after an entry is removed and the list must be redrawn. */
  onRemove: () => void;
};

export function renderUrlList(box: HTMLElement, urls: string[], opts: UrlListOptions): void {
  box.replaceChildren();
  urls.forEach((u, i) => {
    const row = document.createElement("div");
    row.className = "hrow";
    const inp = document.createElement("input");
    inp.value = u;
    inp.placeholder = opts.placeholder;
    inp.spellcheck = false;
    inp.addEventListener("input", () => {
      urls[i] = inp.value;
      opts.onChange();
    });
    const del = document.createElement("button");
    del.className = "btn ghost";
    del.textContent = "×";
    del.title = opts.removeLabel;
    del.setAttribute("aria-label", opts.removeLabel);
    del.addEventListener("click", () => {
      urls.splice(i, 1);
      if (urls.length === 0) urls.push("");
      opts.onChange();
      opts.onRemove();
    });
    row.appendChild(inp);
    row.appendChild(del);
    box.appendChild(row);
  });
}

/** Saved URL lists keep only strings and at most `max` entries. */
export function parseUrlList(raw: string | null, max = 10): string[] {
  try {
    const arr: unknown = raw ? JSON.parse(raw) : null;
    const urls = Array.isArray(arr) ? arr.filter((s): s is string => typeof s === "string").slice(0, max) : [];
    return urls.length > 0 ? urls : [""];
  } catch {
    return [""];
  }
}
