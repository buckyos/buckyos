/* eslint-disable react-refresh/only-export-components -- the instance's property parts and the hook they share */
/* The properties of a control instance (扩展控件 P1 / P2): a composite container or a Cell naming a `def_ref`, edited with
 * its definition's `config_schema` form, named bindings and events. Values are checked here, never by the core (Q15);
 * each change writes only the properties it changes (`entity.set_props`, Q28), guarded by the definition revision the
 * form was built from (Q29). The properties panel (UI 框架 PRD §10.2) shows the parts in their sections: the properties
 * with the type's own, the bindings and the definition under "data", the events under "interaction". */

import { useCallback } from 'react'
import type { BlockDefPayload, BlockDefRead, EntityEnvelope, EventBindings, Json, KeyedContent } from '../../../api/types'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { changedProps, submitProps, type PropKey } from '../../blocks/ops'
import { configProblems } from '../../extensions/controls'
import { EventsForm } from '../../extensions/EventsForm'
import { bindControl } from '../data/binding'
import { bindDialogs } from '../data/dialogBus'
import { InstanceForm } from '../../extensions/InstanceForm'

interface InstancePayload { config?: Record<string, Json>; bindings?: Record<string, { entity_id: string }>; on?: EventBindings; def_ref?: { entity_id: string } }

/** Is the entity an instance of a workspace control definition (a composite, or a Cell with a built definition)? */
export function isInstance(entity: EntityEnvelope, store: ReturnType<typeof useStore>): boolean {
  return (entity.type_id === 'buckyos.container' && entity.kind === 'composite') || (entity.type_id === 'buckyos.cell' && Boolean(entity.def_id && store.outline.get(entity.def_id)?.ident))
}

interface Instance {
  payload: InstancePayload
  def: BlockDefPayload
  defId: string
  config: Record<string, Json>
  bindings: Record<string, string>
  write: (key: PropKey, before: Record<string, Json> | undefined, after: Record<string, Json>, label: string) => void
}

function useInstance(entity: EntityEnvelope, readOnly: string | null): Instance | null {
  const store = useStore()
  const id = entity.entity_id
  const defId = entity.def_id ?? ''
  const own = useLoad(useCallback(() => store.readBatched<KeyedContent<InstancePayload>>(id), [store, id]), useVersion(`e:${id}`))
  const def = useLoad(useCallback(() => (defId ? store.readBatched<BlockDefRead>(defId) : Promise.resolve(null)), [store, defId]), useVersion(`e:${defId}`))
  const payload = own.data?.entity_id === id ? own.data.content.payload : undefined
  const revs = own.data?.content.key_revs ?? {}
  const defPayload = def.data?.entity_id === defId ? def.data.content.payload : undefined
  if (!payload || !defPayload) return null
  const bindings = Object.fromEntries(Object.entries(payload.bindings ?? {}).flatMap(([k, v]) => (Array.isArray(v) ? [] : [[k, v.entity_id]])))
  const write = (key: PropKey, before: Record<string, Json> | undefined, after: Record<string, Json>, label: string) => {
    if (readOnly) return
    void submitProps(store, { entityId: id, key, props: changedProps(before, after), keyRevs: revs, defId, label })
  }
  return { payload, def: defPayload, defId, config: payload.config ?? {}, bindings, write }
}

/** The instance's properties (its definition's `config_schema`). */
export function InstanceConfig({ entity, readOnly }: { entity: EntityEnvelope; readOnly: string | null }) {
  const { t } = useAiwsText()
  const instance = useInstance(entity, readOnly)
  if (!instance) return <div className="aiws-muted">{t('aiworkspace.ext.composite.loading')}</div>
  const problems = configProblems(instance.def, instance.config)
  return (
    <div className="aiws-composite-inspector" data-testid="aiws-instance-inspector" data-instance={entity.entity_id}>
      <fieldset disabled={readOnly !== null} className="aiws-plain-fieldset">
        <InstanceForm def={instance.def} config={instance.config} bindings={instance.bindings} commitOnBlur part="config"
          onConfig={(next) => instance.write('config', instance.config, next, t('aiworkspace.ext.composite.setConfig'))} onBindings={() => undefined} />
      </fieldset>
      {problems.length > 0 && <div className="aiws-error" role="alert">{problems.join('; ')}</div>}
    </div>
  )
}

/** Which definition it is an instance of, its named bindings and the re-bind (the panel's "data" section). */
export function InstanceData({ entity, readOnly }: { entity: EntityEnvelope; readOnly: string | null }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  const instance = useInstance(entity, readOnly)
  if (!instance) return null
  const id = entity.entity_id
  return (
    <div className="aiws-composite-inspector" data-instance={id}>
      <div className="aiws-muted">{t('aiworkspace.ext.composite.instanceOf', { name: instance.def.title ?? instance.def.def_id })}{' '}
        <button type="button" className="aiws-link" data-testid="aiws-instance-open-def" onClick={() => ui.openEntity(instance.defId)}>{t('aiworkspace.ext.composite.openDefinition')}</button></div>
      <fieldset disabled={readOnly !== null} className="aiws-plain-fieldset">
        <InstanceForm def={instance.def} config={instance.config} bindings={instance.bindings} commitOnBlur part="bindings" onConfig={() => undefined}
          onBindings={(next) => instance.write('bindings', instance.payload.bindings as Record<string, Json> | undefined, Object.fromEntries(Object.entries(next).map(([k, v]) => [k, { entity_id: v }])), t('aiworkspace.ext.composite.setBindings'))} />
        {Object.keys(instance.def.bindings ?? {}).length > 0 && (
          <button type="button" className="aiws-link" data-testid="aiws-instance-rebind" onClick={() => void bindControl(store, id, 'rebind', bindDialogs, t('aiworkspace.ext.composite.setBindings')).then((r) => {
            if (r !== 'done' && r !== 'cancelled') store.notify('error', t('aiworkspace.datactl.rejected', { reason: r }))
          })}>{t('aiworkspace.ext.composite.rebind')}</button>
        )}
      </fieldset>
    </div>
  )
}

/** What the instance does on its control's events (the panel's "interaction" section); null when it declares none. */
export function InstanceEvents({ entity, readOnly }: { entity: EntityEnvelope; readOnly: string | null }) {
  const { t } = useAiwsText()
  const instance = useInstance(entity, readOnly)
  if (!instance) return null
  const hasEvents = Object.keys(instance.def.events ?? {}).length > 0 || Object.keys(instance.payload.on ?? {}).length > 0
  if (!hasEvents) return null
  return (
    <EventsForm def={instance.def} instance={entity} on={instance.payload.on ?? {}} bindings={Object.keys(instance.bindings)} hasSource={entity.type_id === 'buckyos.cell' && Boolean(entity.source_id)} readOnly={readOnly}
      onSave={(next) => instance.write('on', instance.payload.on as Record<string, Json> | undefined, next as Record<string, Json>, t('aiworkspace.ext.instance.setEvents'))} />
  )
}
