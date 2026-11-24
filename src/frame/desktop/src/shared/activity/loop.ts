/** Runs `run` now and then every `delay()` ms while `active()`; `kick` runs it again at once. */
export function loop(run: () => Promise<void>, delay: () => number, active: () => boolean = () => true) {
  let timer: ReturnType<typeof setTimeout> | null = null
  let running = false
  let again = false
  let stopped = false
  const schedule = () => {
    if (stopped) return
    if (timer) clearTimeout(timer)
    timer = setTimeout(() => { timer = null; if (active()) void tick(); else schedule() }, delay())
  }
  const tick = async () => {
    if (stopped) return
    if (running) { again = true; return }
    running = true
    try { await run() } finally {
      running = false
      if (again) { again = false; void tick() } else schedule()
    }
  }
  void tick()
  return {
    kick: () => { void tick() },
    stop: () => { stopped = true; if (timer) clearTimeout(timer) },
  }
}
