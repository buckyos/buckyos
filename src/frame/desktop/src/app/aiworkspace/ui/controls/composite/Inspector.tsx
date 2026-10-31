/* The properties of a composite instance (扩展控件 P1): its definition's `config_schema` form and named bindings,
 * stored as overrides on the container; values are checked here, never by the core (Q15). */

import { useCallback } from 'react'
import type { BlockDefRead, EntityEnvelope, Json, KeyedContent } from '../../../api/types'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { configProblems } from '../../extensions/controls'
import { InstanceForm } from '../../extensions/InstanceForm'

interface ContainerPayload { config?: Record<string, Json>; bindings?: Record<string, { entity_id: string }> }

/** A composite's properties and bindings, edited with its definition's form (stored as overrides on the container). */
export function CompositeInspector({ container, readOnly }: { container: EntityEnvelope; readOnly: string | null }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  const id = container.entity_id
  const defId = container.def_id ?? ''
  const own = useLoad(useCallback(() => store.readBatched<KeyedContent<ContainerPayload>>(id), [store, id]), useVersion(`e:${id}`))
  const def = useLoad(useCallback(() => (defId ? store.readBatched<BlockDefRead>(defId) : Promise.resolve(null)), [store, defId]), useVersion(`e:${defId}`))
  const payload = own.data?.entity_id === id ? own.data.content.payload : undefined
  const revs = own.data?.content.key_revs ?? {}
  const defPayload = def.data?.entity_id === defId ? def.data.content.payload : undefined
  if (!payload || !defPayload) return <div className="aiws-muted">{t('aiworkspace.ext.composite.loading')}</div>
  const config = payload.config ?? {}
  const bindings = Object.fromEntries(Object.entries(payload.bindings ?? {}).map(([k, v]) => [k, v.entity_id]))
  const write = (key: 'config' | 'bindings', value: Json | null, label: string) => {
    if (readOnly) return
    void store.submit({ editId: `key:${id}:${key}`, label, operations: [value === null
      ? { op: 'entity.unset_keys', entity_id: id, keys: [{ key, expect: { rev: revs[key] ?? 0 } }] }
      : { op: 'entity.set_keys', entity_id: id, keys: [{ key, value, expect: { rev: revs[key] ?? 0 } }] }] })
  }
  const problems = configProblems(defPayload, config)
  return (
    <div className="aiws-composite-inspector" data-testid="aiws-composite-inspector">
      <div className="aiws-muted">{t('aiworkspace.ext.composite.instanceOf', { name: defPayload.title ?? defPayload.def_id })}{' '}
        <button type="button" className="aiws-link" onClick={() => ui.openEntity(defId)}>{t('aiworkspace.ext.composite.openDefinition')}</button></div>
      <fieldset disabled={readOnly !== null} className="aiws-plain-fieldset">
        <InstanceForm def={defPayload} config={config} bindings={bindings}
          onConfig={(next) => write('config', Object.keys(next).length > 0 ? next as Json : null, t('aiworkspace.ext.composite.setConfig'))}
          onBindings={(next) => write('bindings', Object.keys(next).length > 0 ? Object.fromEntries(Object.entries(next).map(([k, v]) => [k, { entity_id: v }])) as Json : null, t('aiworkspace.ext.composite.setBindings'))} />
      </fieldset>
      {problems.length > 0 && <div className="aiws-error" role="alert">{problems.join('; ')}</div>}
    </div>
  )
}
