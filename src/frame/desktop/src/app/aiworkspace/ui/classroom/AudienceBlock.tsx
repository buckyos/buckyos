/* eslint-disable react-refresh/only-export-components -- a Block definition with its renderer and inspector */
/* Who is here (第四期规划 §5.4, Q17, Q18; the classroom extension's control): bound to the `participants` feed and,
 * optionally, a roster table of the courseware or one imported in the room. Two views: the online list, and the
 * roster against it — present (online and listed), absent (listed, not online), not listed (online). By default only
 * the presenter reads it (the feed is in the presenter's scope); made public, it binds the public projection of the
 * feed — names and presence, no principals — and shows no roster columns. Matching is by DID / user name only. */

import { useEffect, useMemo, useState } from 'react'
import type { Json, KeyedContent, QueryRow, TableSourceContent } from '../../api/types'
import { useLoad, useStore } from '../../state/hooks'
import { useSession } from '../../session/context'
import { useAiwsText } from '../../useAiwsText'
import { aiwsT } from '../../i18n'
import type { BlockDefinition, RenderContext } from '../blocks/registry'
import { cellOp } from '../blocks/ops'
import { TABULAR_ACCEPT } from '@table/files'
import { tableImports } from '../import/tableImportRequest'
import { boundId, ensureBindings, rosterColumns, text, useRows } from './feeds'

export const AUDIENCE_VIEW = 'classroom.audience'

function isPublic(context: RenderContext): boolean {
  return context.payload.config?.public === true
}

function AudienceStatic(context: RenderContext) {
  const store = useStore()
  const session = useSession()
  const { t } = useAiwsText()
  const pub = isPublic(context)
  useEffect(() => { if (!session && context.mode === 'edit') ensureBindings(context, store, [{ name: 'participants', feed: 'participants', public: pub }]) }, [session, context, store, pub])
  const participants = useRows(store, boundId(context, store, session, 'participants'))
  const rosterId = typeof context.payload.config?.roster === 'string' ? context.payload.config.roster as string : undefined
  const roster = useRows(store, pub ? undefined : rosterId)
  const load = useMemo(() => async () => (rosterId && !pub ? await store.session.read<KeyedContent<never> & TableSourceContent>(rosterId).then((r) => r as unknown as { content: TableSourceContent }).catch(() => null) : null), [store, rosterId, pub])
  const table = useLoad(load, rosterId ?? '').data
  const view = context.payload.config?.view === 'roster' && !pub ? 'roster' : 'list'
  const audience = (participants ?? []).filter((p) => text(p, 'role') === 'audience')
  const online = audience.filter((p) => p.values.online === true)
  const presenter = session && (session.role === 'presenter' || session.role === 'owner')
  if (participants === null) {
    return <div className="aiws-classroom aiws-classroom-audience aiws-muted" data-testid={`aiws-audience-${context.cell.entity_id}`}>{t('aiworkspace.session.audience.presenterOnly')}</div>
  }
  const fields: Record<string, string> = {}
  for (const f of table?.content?.fields ?? []) fields[f.field_id] = f.name
  const cols = rosterColumns(roster, fields)
  const listed = (roster ?? []).map((r) => ({ principal: cols.principal ? text(r, cols.principal) : '', name: cols.name ? text(r, cols.name) : '' })).filter((r) => r.principal)
  const onlineIds = new Set(online.map((p) => text(p, 'principal')))
  const listedIds = new Set(listed.map((r) => r.principal))
  const present = listed.filter((r) => onlineIds.has(r.principal))
  const absent = listed.filter((r) => !onlineIds.has(r.principal))
  const extra = online.filter((p) => !listedIds.has(text(p, 'principal')))
  const importRoster = async (file: File) => {
    const bytes = new Uint8Array(await file.arrayBuffer())
    tableImports.open({ source: { kind: 'file', name: file.name, bytes }, step: 'wizard', destination: { folderId: 'data' }, finish: (outcome) => {
      const id = outcome.created[0]
      if (!id || outcome.error) return
      const config: Record<string, Json> = { ...(context.payload.config ?? {}), roster: id, view: 'roster' }
      void store.submit({ editId: `config:${context.cell.entity_id}`, label: t('aiworkspace.session.audience.importRoster'), operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys: [{ key: 'config', value: config, expect: { rev: context.keyRevs.config ?? 0 } }] }] })
    } })
  }
  return (
    <div className="aiws-classroom aiws-classroom-audience" data-testid={`aiws-audience-${context.cell.entity_id}`} data-view={view} data-online={online.length}>
      <div className="aiws-classroom-question">{t('aiworkspace.session.audience.count', { online: online.length, all: audience.length })}</div>
      {view === 'list' ? (
        <ul className="aiws-session-list">
          {audience.map((p) => <li key={p.record_id} data-online={p.values.online === true ? 'true' : 'false'}><span className={`aiws-session-dot${p.values.online === true ? ' is-on' : ''}`} />{text(p, 'name')}</li>)}
        </ul>
      ) : (
        <div className="aiws-classroom-groups">
          <Group title={t('aiworkspace.session.audience.present', { count: present.length })} testId="aiws-audience-present" names={present.map((r) => r.name || r.principal)} />
          <Group title={t('aiworkspace.session.audience.absent', { count: absent.length })} testId="aiws-audience-absent" names={absent.map((r) => r.name || r.principal)} />
          <Group title={t('aiworkspace.session.audience.unlisted', { count: extra.length })} testId="aiws-audience-unlisted" names={extra.map((p: QueryRow) => text(p, 'name'))} />
        </div>
      )}
      {presenter && !session?.replay && !pub && (
        <label className="aiws-link" onPointerDown={(e) => e.stopPropagation()}>{t('aiworkspace.session.audience.importRoster')}
          <input type="file" hidden accept={TABULAR_ACCEPT} data-testid="aiws-audience-import" onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void importRoster(file) }} />
        </label>
      )}
      {!session && context.mode !== 'show' && <div className="aiws-muted aiws-classroom-meta">{pub ? t('aiworkspace.session.audience.publicHint') : t('aiworkspace.session.audience.designHint')}</div>}
    </div>
  )
}

function Group({ title, names, testId }: { title: string; names: string[]; testId: string }) {
  return (
    <div className="aiws-classroom-group" data-testid={testId} data-count={names.length}>
      <b>{title}</b>
      <span>{names.join(', ')}</span>
    </div>
  )
}

function AudienceInspector(context: RenderContext) {
  const store = useStore()
  const { t } = useAiwsText()
  const tables = store.outline.all().filter((e) => e.type_id === 'buckyos.table-source' && !e.source && !e.deleted)
  const config = (context.payload.config ?? {}) as Record<string, Json>
  const [draft, setDraft] = useState({ view: config.view === 'roster' ? 'roster' : 'list', public: config.public === true, roster: typeof config.roster === 'string' ? config.roster : '' })
  const save = () => {
    const next: Record<string, Json> = { ...config, view: draft.view, public: draft.public }
    if (draft.roster) next.roster = draft.roster
    else delete next.roster
    const keys: { key: string; value: Json; expect: { rev: number } }[] = [{ key: 'config', value: next, expect: { rev: context.keyRevs.config ?? 0 } }]
    // public or private is which participants feed the Block reads: bound again on the next render
    if (draft.public !== (config.public === true)) {
      const bindings = { ...(context.payload.bindings ?? {}) } as unknown as Record<string, Json>
      delete bindings.participants
      keys.push({ key: 'bindings', value: bindings, expect: { rev: context.keyRevs.bindings ?? 0 } })
    }
    void store.submit({ editId: `config:${context.cell.entity_id}`, label: t('aiworkspace.session.audience.edit'), operations: [{ op: 'entity.set_keys', entity_id: context.cell.entity_id, keys }] })
  }
  return (
    <div className="aiws-inspector-form" data-testid="aiws-audience-inspector">
      <label>{t('aiworkspace.session.audience.viewLabel')}
        <select value={draft.view} onChange={(e) => setDraft({ ...draft, view: e.target.value })}>
          <option value="list">{t('aiworkspace.session.audience.viewList')}</option>
          <option value="roster">{t('aiworkspace.session.audience.viewRoster')}</option>
        </select>
      </label>
      <label className="aiws-check"><input type="checkbox" checked={draft.public} onChange={(e) => setDraft({ ...draft, public: e.target.checked })} />{t('aiworkspace.session.audience.publicLabel')}</label>
      <label>{t('aiworkspace.session.choice.rosterLabel')}
        <select value={draft.roster} onChange={(e) => setDraft({ ...draft, roster: e.target.value })}>
          <option value="">{t('aiworkspace.session.choice.noRoster')}</option>
          {tables.map((e) => <option key={e.entity_id} value={e.entity_id}>{e.title ?? e.name ?? e.entity_id}</option>)}
        </select>
      </label>
      <button type="button" className="is-primary" onClick={save}>{t('aiworkspace.common.save')}</button>
    </div>
  )
}

export const audienceDefinition: BlockDefinition = {
  type: AUDIENCE_VIEW, version: 1, get title() { return aiwsT('aiworkspace.session.audience.title') }, accepts: [], allowNoSource: true, pureUi: true,
  defaultSize: { w: 320, h: 260 }, cost: { editor: false, html: false },
  Static: AudienceStatic, Inspector: AudienceInspector,
  catalog: { group: 'data', get description() { return aiwsT('aiworkspace.session.audience.description') }, needs: 'none' },
  create: (args) => [cellOp(args, AUDIENCE_VIEW, undefined, { config: { view: 'list', public: false, ...(args.config ?? {}) } })],
}
