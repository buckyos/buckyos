/* Slots shown to each viewer for a while (控件机制 TODO G19, Q11, Q12; manifest 4 `slots.<s>.visible`): from the
 * viewer's first opening of the composite — written once into their user state by the service's clock — for `for_ms`;
 * afterwards the Blocks of that slot are hidden for them only. A presentation, not access control: the shared document
 * keeps everything, design mode shows everything (the frame says "time-limited"), static views, prints and exports do
 * not hide by time. */

import { useEffect, useState, useSyncExternalStore } from 'react'
import { compositeOf, templated } from '@controls/composite'
import type { Laid } from '../../canvas/layout'
import type { CanvasMode } from '../../blocks/registry'
import { useStore } from '../../../state/hooks'
import { serverClock } from '../../../state/serverClock'
import { firstOpenKey } from './Projections'

/** The Blocks of time-limited slots that are hidden for this viewer now. */
export function useTimedHidden(laid: Map<string, Laid>, mode: CanvasMode): ReadonlySet<string> {
  const store = useStore()
  const clock = serverClock(store.session)
  useSyncExternalStore(clock.subscribe, clock.snapshot)
  useSyncExternalStore(store.userState.subscribe, store.userState.snapshot)
  const running = mode !== 'edit' && mode !== 'presentation_edit'
  const rules: { id: string; slots: Set<string>; forMs: number }[] = []
  for (const l of laid.values()) {
    if (l.control?.kind !== 'composite') continue
    const { composition, config } = compositeOf(store.outline, l.entity.entity_id)
    const limited = Object.entries(composition?.slots ?? {}).filter(([, s]) => s.visible?.from === 'first_open')
    if (!limited.length) continue
    rules.push({ id: l.entity.entity_id, slots: new Set(limited.map(([n]) => n)), forMs: Math.max(...limited.map(([, s]) => Number(templated(s.visible!.for_ms, config)) || 0)) })
  }
  const key = rules.map((r) => r.id).join(',')
  const [now, setNow] = useState(() => clock.now())
  useEffect(() => {
    if (!running || !key) return
    // the first opening is written once, by the service's clock (Q12)
    void clock.sync().then(() => {
      for (const id of key.split(',')) if (typeof store.userState.get(firstOpenKey(id)) !== 'number') store.userState.set(firstOpenKey(id), clock.now())
    })
    const timer = window.setInterval(() => setNow(clock.now()), 1000)
    return () => window.clearInterval(timer)
  }, [running, key, clock, store])
  const hidden = new Set<string>()
  if (!running) return hidden
  for (const r of rules) {
    const first = store.userState.get<number>(firstOpenKey(r.id))
    if (typeof first !== 'number' || now < first + r.forMs) continue
    for (const c of store.outline.childrenOf(r.id)) if (c.slot && r.slots.has(c.slot)) hidden.add(c.entity_id)
  }
  return hidden
}
