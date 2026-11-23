export type Timers = {
  set: (callback: () => void, ms: number) => unknown;
  clear: (id: unknown) => void;
};

const browserTimers: Timers = {
  set: (callback, ms) => window.setTimeout(callback, ms),
  clear: (id) => window.clearTimeout(id as number),
};

type Options<T> = {
  run: () => Promise<T>;
  onResult: (result: T) => void;
  onError: (error: unknown) => void;
  /** Stops polling until the next restart. */
  finished: (result: T) => boolean;
  hidden: () => boolean;
  intervalMs: number;
  timers?: Timers;
};

/**
 * Polls one request at a time. `restart` drops whatever is in flight and asks again at once;
 * a hidden page is not polled and is asked again the moment it is shown.
 */
export function createPoller<T>({ run, onResult, onError, finished, hidden, intervalMs, timers = browserTimers }: Options<T>) {
  let active = false;
  let running = false;
  let done = false;
  let generation = 0;
  let timer: unknown = null;

  const schedule = (ms: number) => {
    if (timer !== null) timers.clear(timer);
    timer = timers.set(() => void tick(), ms);
  };

  async function tick() {
    timer = null;
    if (!active || running || done || hidden()) return;
    running = true;
    const started = generation;
    try {
      const result = await run();
      if (started === generation) {
        done = finished(result);
        onResult(result);
      }
    } catch (error) {
      if (started === generation) onError(error);
    }
    running = false;
    if (!active) return;
    if (started !== generation) schedule(0);
    else if (!done) schedule(intervalMs);
  }

  return {
    restart() {
      active = true;
      done = false;
      generation += 1;
      if (!running) schedule(0);
    },
    stop() {
      active = false;
      generation += 1;
      if (timer !== null) timers.clear(timer);
      timer = null;
    },
    visible() {
      if (active && !running && !done && !hidden()) schedule(0);
    },
  };
}
