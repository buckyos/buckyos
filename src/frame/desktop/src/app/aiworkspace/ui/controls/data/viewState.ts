/* State domains of a data control (控件机制 TODO G6, Q6, 05 “UI State 与写入通道”): every setting says where it lives.
 *
 *   shared document   the instance `config` an author saved: the default view everyone opens with
 *   user state        what this person switched to (view, date, filters, folded columns): `block:<id>/view`, theirs only
 *   session           drag previews, open pop-overs, the selection: React state of the body, gone on reload
 *
 * Someone who may change the control can make their view the default ("Set as default": one undoable `entity.set_props`
 * of the changed properties, each guarded by its own version). */

import { useCallback, useMemo } from 'react'
import type { EntityEnvelope, Json } from '../../../api/types'
import { aiwsT } from '../../../i18n'
import type { KeyedContent } from '../../../api/types'
import { useEntity, useStore, useUserState } from '../../../state/hooks'

export const viewKey = (containerId: string) => `block:${containerId}/view`

export interface ControlView<V> {
  /** Defaults ← the instance config ← this person's choices. */
  view: V
  /** The person's own choices (empty: they follow the default). */
  personal: Partial<V>
  /** Change this person's view (null removes a choice). */
  set: (patch: Partial<Record<keyof V, Json | null>>) => void
  /** Back to the instance default. */
  reset: () => void
  /** Save this person's view as the instance default (needs `update` on the control); null when nothing differs. */
  setDefault: (() => void) | null
  container: EntityEnvelope | undefined
  /** May this person change the instance (its default view, its bindings)? */
  canChange: boolean
}

/** The view of a data control: `keys` are the settings that belong to the view (others in `config` are not touched). */
export function useControlView<V extends Record<string, Json | undefined>>(containerId: string, defaults: V, keys: readonly (keyof V & string)[]): ControlView<V> {
  const store = useStore()
  const container = useEntity(containerId)
  const personal = (useUserState<Record<string, Json>>(viewKey(containerId)) ?? {}) as Partial<V>
  const config = (container?.config ?? {}) as Partial<V>
  const layers = JSON.stringify([defaults, config, personal])
  const view = useMemo(() => {
    const [d, c, p] = JSON.parse(layers) as [V, Partial<V>, Partial<V>]
    const out = { ...d } as V
    for (const k of keys) {
      if (c[k] !== undefined) out[k] = c[k] as V[typeof k]
      if (p[k] !== undefined) out[k] = p[k] as V[typeof k]
    }
    return out
  }, [layers, keys])
  const set = useCallback((patch: Partial<Record<keyof V, Json | null>>) => {
    const next: Record<string, Json> = { ...(store.userState.get<Record<string, Json>>(viewKey(containerId)) ?? {}) }
    for (const [k, v] of Object.entries(patch)) {
      if (v === null || v === undefined) delete next[k]
      else next[k] = v as Json
    }
    store.userState.set(viewKey(containerId), Object.keys(next).length ? next : null)
  }, [store, containerId])
  const reset = useCallback(() => store.userState.set(viewKey(containerId), null), [store, containerId])
  const canChange = Boolean(container?.capabilities.includes('update'))
  const differs = keys.filter((k) => personal[k] !== undefined && JSON.stringify(personal[k]) !== JSON.stringify(config[k] ?? defaults[k]))
  const setDefault = canChange && differs.length > 0 ? () => {
    const props: Record<string, Json | null> = {}
    for (const k of differs) props[k] = (JSON.stringify(personal[k]) === JSON.stringify(defaults[k]) ? null : personal[k]) as Json | null
    void store.session.read<KeyedContent<Record<string, Json>>>(containerId).then((read) => store.submit({
      editId: `config:${containerId}`, label: aiwsT('aiworkspace.datactl.setDefault'),
      operations: [{ op: 'entity.set_props', entity_id: containerId, key: 'config', props, expect: Object.fromEntries(Object.keys(props).map((name) => [name, read.content.key_revs[`p:config.${name}`] ?? 0])) }],
    })).then((outcome) => {
      // the person now follows the default they set
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') set(Object.fromEntries(differs.map((k) => [k, null])) as Partial<Record<keyof V, Json | null>>)
    })
  } : null
  return { view, personal, set, reset, setDefault, container, canChange }
}
