import type { Timers } from "./poller";

export const CHECK_AFTER_MS = 2 * 60 * 1000;
export const FAIL_AFTER_MS = 5 * 60 * 1000;
export const CHECK_INTERVAL_MS = 5 * 1000;

/** Which layer is missing: the node itself, its public DNS, or the certificate / forwarding behind it. */
export type StartupCause = "node" | "dns" | "service";

export type StartupState =
  | { phase: "starting" }
  | { phase: "checking" }
  | { phase: "ready" }
  | { phase: "failed"; cause: StartupCause }
  | { phase: "unverifiable"; nodeUp: boolean };

type Options = {
  /** When `commit_active` returned, in wall-clock milliseconds. */
  activatedAt: number;
  /** False for an uploaded certificate that public clients do not trust. */
  certificateTrusted: boolean;
  probe: () => Promise<boolean>;
  diagnose: () => Promise<StartupCause>;
  onChange: (state: StartupState) => void;
  now?: () => number;
  timers?: Timers;
};

/**
 * Nothing for two minutes, then one probe every five seconds until it succeeds or five minutes have
 * passed. Timers only wake it up: the phase always follows the clock, so a throttled background tab
 * or a sleeping laptop does not stretch the schedule.
 */
export function createStartupMonitor({
  activatedAt,
  certificateTrusted,
  probe,
  diagnose,
  onChange,
  now = Date.now,
  timers = {
    set: (callback, ms) => window.setTimeout(callback, ms),
    clear: (id) => window.clearTimeout(id as number),
  },
}: Options) {
  let state: StartupState = { phase: "starting" };
  let stopped = false;
  let running = false;
  let timer: unknown = null;

  const elapsed = () => now() - activatedAt;
  const move = (next: StartupState) => {
    state = next;
    if (!stopped) onChange(next);
  };
  const schedule = (ms: number) => {
    if (timer !== null) timers.clear(timer);
    timer = timers.set(() => void tick(), Math.max(ms, 0));
  };

  const settle = async () => {
    const cause = await diagnose().catch((): StartupCause => "service");
    if (stopped) return;
    move(
      certificateTrusted
        ? { phase: "failed", cause }
        : { phase: "unverifiable", nodeUp: cause !== "node" },
    );
  };

  const attempt = () => probe().catch(() => false);

  async function tick() {
    timer = null;
    if (stopped || running || (state.phase !== "starting" && state.phase !== "checking")) return;
    if (elapsed() < CHECK_AFTER_MS) {
      schedule(CHECK_AFTER_MS - elapsed());
      return;
    }
    if (state.phase === "starting") move({ phase: "checking" });
    running = true;
    const startedAt = now();
    const ok = await attempt();
    if (stopped) return;
    if (ok) move({ phase: "ready" });
    else if (elapsed() >= FAIL_AFTER_MS) await settle();
    else schedule(startedAt + CHECK_INTERVAL_MS - now());
    running = false;
  }

  return {
    start() {
      stopped = false;
      onChange(state);
      schedule(0);
    },
    stop() {
      stopped = true;
      if (timer !== null) timers.clear(timer);
      timer = null;
    },
    /** The tab came back: timers may have been held, the clock was not. */
    wake() {
      if (!stopped && !running && timer !== null) schedule(0);
    },
    async recheck() {
      if (stopped || running || (state.phase !== "failed" && state.phase !== "unverifiable")) return;
      running = true;
      move({ phase: "checking" });
      const ok = await attempt();
      if (!stopped) {
        if (ok) move({ phase: "ready" });
        else await settle();
      }
      running = false;
    },
  };
}
