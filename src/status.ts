export type StatusRenderer = () => string;

const statusRenderers = new Map<string, StatusRenderer>();

export function setStatusText(id: string, render: StatusRenderer): void {
  const el = document.getElementById(id);
  if (el) el.textContent = render();
  statusRenderers.set(id, render);
}

export function refreshStatusTexts(): void {
  statusRenderers.forEach((render, id) => {
    const el = document.getElementById(id);
    if (el) el.textContent = render();
  });
}
