import { visibleRange } from "./results-model.ts";

const OVERSCAN = 12;
const DEFAULT_ROW_HEIGHT = 31;

/**
 * Renders only the rows of a scrolling table that are in view; spacer rows
 * keep the full scroll height, so lists of any length stay fast. Rows must
 * be single-line, which the tables guarantee with `white-space: nowrap`.
 */
export class VirtualRows<T> {
  private items: T[] = [];
  private rowHeight = DEFAULT_ROW_HEIGHT;
  private measured = false;
  private range: [number, number] = [-1, -1];
  private frame = 0;

  constructor(
    private wrap: HTMLElement,
    private body: HTMLTableSectionElement,
    private cols: number,
    private renderRow: (item: T, index: number) => HTMLTableRowElement,
  ) {
    wrap.addEventListener("scroll", () => this.schedule(), { passive: true });
    if (typeof ResizeObserver === "function") {
      new ResizeObserver(() => this.schedule()).observe(wrap);
    }
  }

  /** Show a new list; keeps the scroll position when `keepScroll` is set. */
  set(items: T[], keepScroll = true): void {
    this.items = items;
    if (!keepScroll) this.wrap.scrollTop = 0;
    this.repaint();
  }

  /** Re-render the visible rows, e.g. after the selection changed. */
  repaint(): void {
    this.range = [-1, -1];
    this.paint();
  }

  /** Scroll just enough to bring row `i` into view below the sticky header. */
  reveal(i: number): void {
    const head = this.headHeight();
    const top = head + i * this.rowHeight;
    if (top - head < this.wrap.scrollTop) {
      this.wrap.scrollTop = top - head;
    } else if (top + this.rowHeight > this.wrap.scrollTop + this.wrap.clientHeight) {
      this.wrap.scrollTop = top + this.rowHeight - this.wrap.clientHeight;
    }
  }

  private headHeight(): number {
    return this.body.parentElement?.querySelector("thead")?.getBoundingClientRect().height ?? 0;
  }

  private schedule(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.paint();
    });
  }

  private spacer(height: number): HTMLTableRowElement {
    const tr = document.createElement("tr");
    tr.className = "spacer";
    tr.setAttribute("aria-hidden", "true");
    const td = document.createElement("td");
    td.colSpan = this.cols;
    td.style.height = `${height}px`;
    tr.appendChild(td);
    return tr;
  }

  private paint(): void {
    const head = this.headHeight();
    const [first, last] = visibleRange(
      Math.max(0, this.wrap.scrollTop - head),
      this.wrap.clientHeight,
      this.rowHeight,
      this.items.length,
      OVERSCAN,
    );
    if (first === this.range[0] && last === this.range[1]) return;
    this.range = [first, last];
    const frag = document.createDocumentFragment();
    if (first > 0) frag.appendChild(this.spacer(first * this.rowHeight));
    for (let i = first; i < last; i++) frag.appendChild(this.renderRow(this.items[i], i));
    if (last < this.items.length) frag.appendChild(this.spacer((this.items.length - last) * this.rowHeight));
    this.body.replaceChildren(frag);

    // Spacer heights use the real row height once a row is on screen.
    if (!this.measured && last > first) {
      const row = this.body.querySelector("tr:not(.spacer)");
      const h = row?.getBoundingClientRect().height ?? 0;
      if (h > 0) {
        this.measured = true;
        if (Math.abs(h - this.rowHeight) > 0.5) {
          this.rowHeight = h;
          this.repaint();
        }
      }
    }
  }
}
