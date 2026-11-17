/* When the printed pages are done (打印 TODO Q18): `media.page` prints once `window.__AIWS_PRINT_READY__` is set. A
 * part of a page that finishes on its own time (an HTML Block running its code) holds the pages until it is done;
 * the rest is watched in the DOM: no Block still loading, every picture decoded, the fonts loaded and the DOM quiet
 * for a moment. Everything has a deadline: what is not done by then prints as it is. */

const holds = new Set<symbol>()
const listeners = new Set<() => void>()

function changed() {
  for (const listener of [...listeners]) listener()
}

/** Hold the pages until the returned function is called (once; later calls do nothing). */
export function holdPrint(): () => void {
  const token = Symbol('print-hold')
  holds.add(token)
  return () => { if (holds.delete(token)) changed() }
}

/** What is still loading in `root`: a Block reading its content, a picture that is not decoded, a part that marks itself. */
const LOADING = '[data-testid^="aiws-block-loading-"], .aiws-block-loading, .aiws-media-loading, [data-print-pending]'

function pictures(root: HTMLElement): HTMLImageElement[] {
  return [...root.querySelectorAll('img')]
}

function busy(root: HTMLElement): boolean {
  if (holds.size > 0) return true
  if (root.querySelector(LOADING)) return true
  if (pictures(root).some((img) => !img.complete)) return true
  return document.fonts.status !== 'loaded'
}

const frame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
const sleep = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

/** Resolves once nothing in `root` is busy and its DOM has not changed for `quietMs`, or at `deadline` (performance.now()). */
export async function settled(root: HTMLElement, deadline: number, quietMs = 250): Promise<boolean> {
  let lastChange = performance.now()
  const observer = new MutationObserver(() => { lastChange = performance.now() })
  observer.observe(root, { subtree: true, childList: true, attributes: true, characterData: true })
  const wake = () => { lastChange = performance.now() }
  listeners.add(wake)
  try {
    for (;;) {
      if (performance.now() >= deadline) return false
      await frame()
      if (busy(root)) { await sleep(30); continue }
      if (performance.now() - lastChange < quietMs) { await sleep(Math.min(quietMs, 50)); continue }
      // a decoded picture lays out at its size: one more look after decoding
      await Promise.all(pictures(root).map((img) => img.decode().catch(() => undefined)))
      await document.fonts.ready
      await frame()
      if (!busy(root) && performance.now() - lastChange >= quietMs) return true
    }
  } finally {
    observer.disconnect()
    listeners.delete(wake)
  }
}
