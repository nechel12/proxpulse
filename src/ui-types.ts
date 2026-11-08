/** The translation function every panel gets (`t` in main.ts). */
export type Translate = (key: string, params?: Record<string, string | number>) => string;

/** Sets a status line's text, re-rendered on a language change. */
export type StatusSetter = (id: string, render: () => string) => void;
