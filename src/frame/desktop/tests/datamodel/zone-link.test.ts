/// <reference types="vite/client" />
import { buckyos } from 'buckyos'

const page = Object.assign(new EventTarget(), { visibilityState: 'visible' })
Object.assign(globalThis, { document: page })
const { isZoneReachable, onZoneRecovered, subscribeZoneEvents, subscribeZoneReachable, watchZoneLink } = await import('../../src/api/zone_link.ts')
delete (globalThis as { document?: unknown }).document

type Open = (patterns: string[]) => Promise<{ pullEvent(): Promise<unknown>; close(): Promise<void> }>

function equal(actual: unknown, expected: unknown, label: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`${label}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`)
}

class Clock {
  now = 1_000_000
  private timers = new Map<number, { at: number; run: () => void }>()
  private next = 1
  private readonly real = { setTimeout: globalThis.setTimeout, clearTimeout: globalThis.clearTimeout, now: Date.now }

  install() {
    Date.now = () => this.now
    globalThis.setTimeout = ((run: () => void, ms = 0) => { this.timers.set(this.next, { at: this.now + ms, run }); return this.next++ }) as unknown as typeof setTimeout
    globalThis.clearTimeout = ((id: number) => { this.timers.delete(id) }) as unknown as typeof clearTimeout
  }

  uninstall() {
    Date.now = this.real.now
    globalThis.setTimeout = this.real.setTimeout
    globalThis.clearTimeout = this.real.clearTimeout
  }

  async settle() { for (let turn = 0; turn < 50; turn++) await Promise.resolve() }

  async advance(ms: number) {
    const end = this.now + ms
    for (;;) {
      await this.settle()
      const due = [...this.timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0]
      if (!due) break
      this.timers.delete(due[0])
      this.now = Math.max(this.now, due[1].at)
      due[1].run()
    }
    this.now = end
    await this.settle()
  }
}

interface Zone {
  clock: Clock
  mode: 'up' | 'down' | 'hang' | 'refuse'
  opened: string[][]
  live(): number
  cut(): void
  emit(event: unknown): void
}

function zoneTest(name: string, run: (zone: Zone) => Promise<void>) {
  Deno.test(name, async () => {
    const clock = new Clock()
    const readers = new Set<{ push(value: unknown): void }>()
    const zone: Zone = {
      clock, mode: 'up', opened: [],
      live: () => readers.size,
      cut: () => { for (const reader of [...readers]) reader.push(null) },
      emit: (event) => readers.forEach((reader) => reader.push(event)),
    }
    const open: Open = async (patterns) => {
      zone.opened.push(patterns)
      if (zone.mode === 'hang') return new Promise(() => undefined)
      if (zone.mode === 'down') throw new TypeError('Failed to fetch')
      if (zone.mode === 'refuse') throw new Error('kevent stream request failed: 502')
      const queue: unknown[] = []
      let wake: (() => void) | null = null
      const reader = { push: (value: unknown) => { queue.push(value); wake?.() } }
      readers.add(reader)
      return {
        pullEvent: async () => {
          while (queue.length === 0) await new Promise<void>((resolve) => { wake = resolve })
          return queue.shift()
        },
        close: async () => { readers.delete(reader); reader.push(null) },
      }
    }
    const saved = buckyos.createEventReader
    buckyos.createEventReader = open as unknown as typeof buckyos.createEventReader
    Object.assign(globalThis, { window: globalThis, document: page })
    clock.install()
    try {
      await run(zone)
      await clock.advance(20_000)
      equal(zone.live(), 0, 'streams left open')
      equal(isZoneReachable(), true, 'reachable once nothing is followed')
    } finally {
      clock.uninstall()
      buckyos.createEventReader = saved
      delete (globalThis as { window?: unknown }).window
      delete (globalThis as { document?: unknown }).document
    }
  })
}

zoneTest('loading and a short interruption do not show; an outage does, until the Zone answers', async (zone) => {
  const { clock } = zone
  const changes: boolean[] = []
  let recovered = 0
  const stop = [subscribeZoneReachable(() => changes.push(isZoneReachable())), onZoneRecovered(() => recovered++), watchZoneLink()]
  equal(isZoneReachable(), true, 'while loading')
  await clock.advance(0)
  equal([zone.live(), recovered], [1, 0], 'connected')

  zone.cut()
  await clock.advance(2_000)
  equal([zone.live(), recovered, changes], [1, 1, []], 'after a short interruption')

  zone.mode = 'down'
  zone.cut()
  await clock.advance(4_000)
  equal(isZoneReachable(), true, 'within the grace period')
  await clock.advance(3_000)
  equal(changes, [false], 'after the grace period')
  zone.mode = 'up'
  await clock.advance(1_500)
  equal([changes, recovered], [[false, true], 2], 'after the outage')
  stop.forEach((release) => release())
})

zoneTest('a page that loads without its Zone says so after the grace period, also when nothing answers at all', async (zone) => {
  zone.mode = 'hang'
  const stop = watchZoneLink()
  await zone.clock.advance(9_000)
  equal(isZoneReachable(), true, 'still connecting')
  await zone.clock.advance(2_000)
  equal(isZoneReachable(), false, 'no answer')
  stop()
})

zoneTest('a refusal is an answer: the Zone is reachable while its kevent service is not', async (zone) => {
  const stop = watchZoneLink()
  await zone.clock.advance(0)
  zone.mode = 'refuse'
  zone.cut()
  await zone.clock.advance(30_000)
  equal(isZoneReachable(), true, 'kevent down')
  let recovered = 0
  const stopRecovered = onZoneRecovered(() => recovered++)
  zone.mode = 'up'
  await zone.clock.advance(1_500)
  equal(recovered, 1, 'events were missed')
  stopRecovered()
  stop()
})

zoneTest('what a hidden page lost counts only once it is visible', async (zone) => {
  const show = (visibilityState: string) => { page.visibilityState = visibilityState; page.dispatchEvent(new Event('visibilitychange')) }
  const stop = watchZoneLink()
  await zone.clock.advance(0)
  show('hidden')
  zone.mode = 'down'
  zone.cut()
  await zone.clock.advance(60_000)
  equal(isZoneReachable(), true, 'hidden')
  show('visible')
  await zone.clock.advance(4_000)
  equal(isZoneReachable(), true, 'just visible again')
  await zone.clock.advance(3_000)
  equal(isZoneReachable(), false, 'still out of reach')
  stop()
})

zoneTest('the link opens a stream of its own only while the page follows nothing', async (zone) => {
  const { clock } = zone
  const release = watchZoneLink()
  await clock.advance(0)
  equal([zone.live(), zone.opened.length], [1, 1], 'own stream')
  const events: unknown[] = []
  const stop = subscribeZoneEvents(['/obj/msg-center/a'], (event) => { events.push(event); throw Error('handler') })
  await clock.advance(0)
  equal([zone.live(), zone.opened.at(-1)], [1, ['/obj/msg-center/a']], 'followed stream')
  zone.emit({ eventid: 'one' })
  zone.emit({ eventid: 'two' })
  await clock.advance(0)
  equal(events.length, 2, 'events after a handler failed')
  stop()
  await clock.advance(0)
  equal([zone.live(), zone.opened.length], [1, 3], 'own stream again')
  release()
})

zoneTest('streams that drop together recover once', async (zone) => {
  let recovered = 0
  const stop = [onZoneRecovered(() => recovered++), subscribeZoneEvents(['/a'], () => undefined), subscribeZoneEvents(['/b'], () => undefined)]
  await zone.clock.advance(0)
  zone.cut()
  await zone.clock.advance(3_000)
  equal([zone.live(), recovered], [2, 1], 'recovered')
  globalThis.dispatchEvent(new Event('online'))
  equal(recovered, 2, 'the system reports a network')
  stop.forEach((release) => release())
})
