import type { ZoneEventReader } from './zone_link.ts'

let down = false
const ends = new Set<() => void>()

/** An event stream of the mock Zone; `window.__mockZoneLink.setDown` cuts it. */
export async function openMockEventReader(): Promise<ZoneEventReader> {
  if (down) throw new TypeError('the mock Zone is down')
  let end!: () => void
  const ended = new Promise<null>((resolve) => { end = () => resolve(null) })
  ends.add(end)
  return { pullEvent: () => ended, close: async () => { ends.delete(end); end() } }
}

if (import.meta.env?.DEV && typeof window !== 'undefined') {
  Object.assign(window, { __mockZoneLink: { setDown(value: boolean) { down = value; if (value) ends.forEach((end) => end()) } } })
}
