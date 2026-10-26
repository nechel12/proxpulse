// CSV escaping shared by export. Values from untrusted proxy lists may start
// with spreadsheet formula characters, so guard them as text.
export function csvCell(v: unknown): string {
  const s = v == null ? "" : String(v);
  const guarded = /^[=+\-@\t\r]/.test(s) ? `'${s}` : s;
  return /[",\n\r]/.test(guarded) ? `"${guarded.replace(/"/g, '""')}"` : guarded;
}
