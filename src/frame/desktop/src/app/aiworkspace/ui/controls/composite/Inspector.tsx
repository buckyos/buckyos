/* The properties of a control instance (扩展控件 P1 / P2): a composite container or a Cell naming a `def_ref`, edited with
 * its definition's `config_schema` form, named bindings and events. Values are checked here, never by the core (Q15);
 * each change writes only the properties it changes (`entity.set_props`, Q28), guarded by the definition revision the
 * form was built from (Q29). */

import { useCallback, useState } from 'react'
import type { BlockDefRead, EntityEnvelope, EventBindings, Json, KeyedContent } from '../../../api/types'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { changedProps, submitProps, type PropKey } from '../../blocks/ops'
import { configProblems } from '../../extensions/controls'
import { EventsForm } from '../../extensions/EventsForm'
import { InstanceForm } from '../../extensions/InstanceForm'

interface InstancePayload { config?: Record<string, Json>; bindings?: Record<string, { entity_id: string }>; on?: EventBindings; def_ref?: { entity_id: string } }

/** A control instance's properties, bindings and events (stored as overrides on the instance). */
export function InstanceInspector({ entity, readOnly }: { entity: EntityEnvelope; readOnly: string | null }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t } = useAiwsText()
  const [page, setPage] = useState<'props' | 'events'>('props')
  const id = entity.entity_id
  const defId = entity.def_id ?? ''
  const own = useLoad(useCallback(() => store.readBatched<KeyedContent<InstancePayload>>(id), [store, id]), useVersion(`e:${id}`))
  const def = useLoad(useCallback(() => (defId ? store.readBatched<BlockDefRead>(defId) : Promise.resolve(null)), [store, defId]), useVersion(`e:${defId}`))
  const payload = own.data?.entity_id === id ? own.data.content.payload : undefined
  const revs = own.data?.content.key_revs ?? {}
  const defPayload = def.data?.entity_id === defId ? def.data.content.payload : undefined
  if (!payload || !defPayload) return <div className="aiws-muted">{t('aiworkspace.ext.composite.loading')}</div>
  const config = payload.config ?? {}
  const bindings = Object.fromEntries(Object.entries(payload.bindings ?? {}).map(([k, v]) => [k, v.entity_id]))
  const write = (key: PropKey, before: Record<string, Json> | undefined, after: Record<string, Json>, label: string) => {
    if (readOnly) return
    void submitProps(store, { entityId: id, key, props: changedProps(before, after), keyRevs: revs, defId, label })
  }
  const problems = configProblems(defPayload, config)
  const hasEvents = Object.keys(defPayload.events ?? {}).length > 0 || Object.keys(payload.on ?? {}).length > 0
  const form = (
    <fieldset disabled={readOnly !== null} className="aiws-plain-fieldset">
      <InstanceForm def={defPayload} config={config} bindings={bindings} commitOnBlur
        onConfig={(next) => write('config', config, next, t('aiworkspace.ext.composite.setConfig'))}
        onBindings={(next) => write('bindings', payload.bindings as Record<string, Json> | undefined, Object.fromEntries(Object.entries(next).map(([k, v]) => [k, { entity_id: v }])), t('aiworkspace.ext.composite.setBindings'))} />
    </fieldset>
  )
  return (
    <div className="aiws-composite-inspector" data-testid="aiws-instance-inspector" data-instance={id}>
      <div className="aiws-muted">{t('aiworkspace.ext.composite.instanceOf', { name: defPayload.title ?? defPayload.def_id })}{' '}
        <button type="button" className="aiws-link" onClick={() => ui.openEntity(defId)}>{t('aiworkspace.ext.composite.openDefinition')}</button></div>
      {hasEvents && (
        <div className="aiws-tabs" role="tablist" aria-label={t('aiworkspace.ext.instance.pages')}>
          <button type="button" role="tab" aria-selected={page === 'props'} data-testid="aiws-instance-page-props" onClick={() => setPage('props')}>{t('aiworkspace.ext.instance.props')}</button>
          <button type="button" role="tab" aria-selected={page === 'events'} data-testid="aiws-instance-page-events" onClick={() => setPage('events')}>{t('aiworkspace.ext.instance.events')}</button>
        </div>
      )}
      {page === 'events' && hasEvents
        ? <EventsForm def={defPayload} instance={entity} on={payload.on ?? {}} bindings={Object.keys(bindings)} hasSource={entity.type_id === 'buckyos.cell' && Boolean(entity.source_id)} readOnly={readOnly}
            onSave={(next) => write('on', payload.on as Record<string, Json> | undefined, next as Record<string, Json>, t('aiworkspace.ext.instance.setEvents'))} />
        : form}
      {problems.length > 0 && <div className="aiws-error" role="alert">{problems.join('; ')}</div>}
    </div>
  )
}
