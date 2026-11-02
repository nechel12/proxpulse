import { Channel, invoke } from "@tauri-apps/api/core";
import type { SpeedResult } from "./types.ts";

export const SPEED_SIZES_MB = [1, 5, 10, 25];
const STORAGE_KEY = "pp-speed-mb";

export function loadSpeedMb(): number {
  try {
    const v = Number(localStorage.getItem(STORAGE_KEY));
    return SPEED_SIZES_MB.includes(v) ? v : 5;
  } catch {
    return 5;
  }
}

export function saveSpeedMb(mb: number): void {
  try {
    localStorage.setItem(STORAGE_KEY, String(mb));
  } catch { /* ignore */ }
}

export function formatMbps(v: number): string {
  return `${v.toFixed(v < 10 ? 2 : 1)} Mbit/s`;
}

/** Table label: the speed, or ✗ for a failed measurement. */
export function speedLabel(s: SpeedResult | undefined): string {
  if (!s) return "—";
  return s.ok && s.mbps != null ? formatMbps(s.mbps) : "✗";
}

/** Measure download speed through each proxy; results arrive one by one. */
export async function runSpeedTest(
  targets: { proxy: string; url: string }[],
  mb: number,
  onResult: (r: SpeedResult) => void,
): Promise<void> {
  const channel = new Channel<SpeedResult>();
  channel.onmessage = onResult;
  await invoke("speed_test", { targets, bytes: mb * 1_000_000, onResult: channel });
}
