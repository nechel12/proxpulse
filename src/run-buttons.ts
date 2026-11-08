/** Who holds the checker, as the run controls see it. */
export type RunState = {
  /** A manual check or speed test runs. */
  manual: boolean;
  /** Start was pressed and the check is being prepared. */
  starting: boolean;
  /** A scheduled check holds the checker. */
  autoChecking: boolean;
  /** A schedule is fetching subscriptions or checking. */
  autoRunning: boolean;
};

/**
 * Which of Start, Stop and "Check now" work. One checker serves the manual
 * check and every pool's schedule, so a run of either kind locks Start.
 */
export function runButtons(s: RunState): { start: boolean; stop: boolean; autoNow: boolean } {
  return {
    start: !(s.manual || s.starting || s.autoChecking),
    stop: s.manual || s.autoRunning,
    autoNow: !s.autoChecking,
  };
}
