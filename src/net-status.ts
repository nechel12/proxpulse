type Translate = (key: string, params?: Record<string, string | number>) => string;

export class NetStatus {
  private key = "net_idle";
  private params: Record<string, string | number> | undefined;
  private dot = "idle";

  set(
    state: "idle" | "work" | "done",
    key: string,
    params?: Record<string, string | number>,
  ): void {
    this.dot = state;
    this.key = key;
    this.params = params;
  }

  render(t: Translate): void {
    const dot = document.getElementById("net-dot");
    if (dot) dot.className = "dot " + this.dot;
    const text = document.getElementById("net-text");
    if (text) text.textContent = t(this.key, this.params);
  }
}
