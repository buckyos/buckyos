/* Relations and dependencies of one entity (phase two §6.2): what it references, who uses it,
 * what generated it and what it generated, with the freshness the core computed. Unreadable
 * referrers are one line, never names or counts. "Locate on canvas" lists the Blocks that show it. */

import { useCallback } from 'react'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import type { RelationsInfo } from '../../api/types'
import { localizedLabels } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { FRESHNESS_LABEL, freshnessDetail } from './freshnessText'
import { TYPE_LABEL } from './dataOps'

const KIND_LABEL: Readonly<Record<string, string>> = localizedLabels({
  bind: 'aiworkspace.data.relations.kind.bind', embed: 'aiworkspace.data.relations.kind.embed', value: 'aiworkspace.data.relations.kind.value',
  body: 'aiworkspace.data.relations.kind.body', anchor: 'aiworkspace.data.relations.kind.anchor', input: 'aiworkspace.data.relations.kind.input',
  derived: 'aiworkspace.data.relations.kind.derived', produced: 'aiworkspace.data.relations.kind.produced', def: 'aiworkspace.data.relations.kind.def',
  asset: 'aiworkspace.data.relations.kind.asset', connector_endpoint: 'aiworkspace.data.relations.kind.connectorEndpoint',
  show_target: 'aiworkspace.data.relations.kind.showTarget', show_surface: 'aiworkspace.data.relations.kind.showSurface', def_dep: 'aiworkspace.data.relations.kind.defDep',
})

export function RelationsPanel({ entityId }: { entityId: string }) {
  const { t, tNode } = useAiwsText()
  const store = useStore()
  const ui = useWorkspaceUi()
  const version = useVersion('any')
  const load = useCallback(() => store.session.relations(entityId), [store, entityId])
  const read = useLoad<RelationsInfo>(load, version)
  if (read.error && !read.data) return <div className="aiws-error" role="alert">{t('aiworkspace.data.relations.readError', { error: read.error })}</div>
  if (!read.data) return <div className="aiws-muted">{t('aiworkspace.data.relations.loading')}</div>
  const r = read.data
  const label = (b: { title?: string | null; name: string | null; entity_id: string } | undefined, id: string) => b?.title ?? b?.name ?? id
  const locate = (blockId: string) => ui.openEntity(blockId)
  const freshness = r.freshness
  return (
    <div className="aiws-relations" data-testid={`aiws-relations-${entityId}`}>
      {freshness && freshness.status !== 'none' && (
        <div className={`aiws-relations-freshness${freshness.status === 'current' ? '' : ' aiws-warning'}`} data-testid="aiws-relations-freshness" data-status={freshness.status}>
          <b>{FRESHNESS_LABEL[freshness.status]}</b> · {freshnessDetail(freshness)}
          {freshness.manual_modified && <div>{t('aiworkspace.data.relations.manual', { content: freshness.content_rev ?? '', generated: freshness.generated_rev ?? '' })}</div>}
          {freshness.wish_id && <div className="aiws-muted">{tNode(freshness.simulated ? 'aiworkspace.data.relations.generatedBySimulated' : 'aiworkspace.data.relations.generatedBy', {
            wish: <button type="button" className="aiws-link" onClick={() => ui.openEntity(freshness.wish_id ?? '')}>{label(store.outline.get(freshness.wish_id ?? ''), freshness.wish_id ?? '')}</button>,
            run: freshness.run_id, executor: freshness.executor,
          })}</div>}
          {(freshness.upstream ?? []).length > 0 && <div>{tNode('aiworkspace.data.relations.upstream', {
            items: freshness.upstream?.map((u) => <button key={u.entity_id} type="button" className="aiws-link" onClick={() => ui.openEntity(u.entity_id)}>{t('aiworkspace.data.relations.upstreamItem', { name: label(store.outline.get(u.entity_id), u.entity_id), status: FRESHNESS_LABEL[u.status] })}</button>),
          })}</div>}
        </div>
      )}
      <div className="aiws-panel-title">{t('aiworkspace.data.relations.outgoing')} <span className="aiws-muted">{r.outgoing.length}</span></div>
      {r.outgoing.length === 0 && <div className="aiws-muted">{t('aiworkspace.data.relations.noOutgoing')}</div>}
      <ul className="aiws-relation-list">
        {r.outgoing.map((line, i) => (
          <li key={i} data-testid="aiws-relation-out" data-kind={line.kind}>
            <span className="aiws-outline-type">{KIND_LABEL[line.kind] ?? line.kind}</span>
            {line.external ? <span>{t('aiworkspace.data.relations.external', { workspace: line.workspace_id ?? '', entity: line.entity_id })}</span>
              : line.object_id ? <span className="aiws-muted">{t('aiworkspace.data.relations.object', { id: line.object_id.slice(0, 20) })}</span>
                : line.readable === false ? <span className="aiws-muted">{t('aiworkspace.data.relations.unreadable')}</span>
                  : line.missing ? <span className="aiws-error">{t('aiworkspace.data.relations.missing', { id: line.entity_id })}</span>
                    : <button type="button" className="aiws-link" onClick={() => ui.openEntity(line.entity_id)}>{line.target?.deleted ? t('aiworkspace.data.relations.deleted', { name: label(line.target, line.entity_id) }) : label(line.target, line.entity_id)}</button>}
          </li>
        ))}
      </ul>
      <div className="aiws-panel-title">{t('aiworkspace.data.relations.incoming')} <span className="aiws-muted">{r.incoming.length}</span></div>
      {r.incoming.length === 0 && !r.hidden_incoming && <div className="aiws-muted">{t('aiworkspace.data.relations.noIncoming')}</div>}
      <ul className="aiws-relation-list">
        {r.incoming.map((line, i) => (
          <li key={i} data-testid="aiws-relation-in" data-kind={line.kind}>
            <span className="aiws-outline-type">{KIND_LABEL[line.kind] ?? line.kind}</span>
            <button type="button" className="aiws-link" onClick={() => (line.source?.type_id === 'buckyos.cell' ? locate(line.entity_id) : ui.openEntity(line.entity_id))}>{label(line.source, line.entity_id)}</button>
            <span className="aiws-muted"> {TYPE_LABEL[line.source?.type_id ?? ''] ?? ''}{line.blocks_delete ? t('aiworkspace.data.relations.blocksDelete') : ''}</span>
          </li>
        ))}
      </ul>
      {r.hidden_incoming && <div className="aiws-muted" data-testid="aiws-relation-hidden">{t('aiworkspace.data.relations.hidden')}</div>}
      {r.blocks.length > 0 && (
        <div className="aiws-panel-title">{t('aiworkspace.data.relations.locate')} <span className="aiws-muted">{t('aiworkspace.data.relations.blockCount', { count: r.blocks.length })}</span></div>
      )}
      {r.blocks.map((b) => {
        const surface = store.outline.ancestors(b.entity_id).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')
        return <button key={b.entity_id} type="button" className="aiws-link" data-testid={`aiws-locate-${b.entity_id}`} onClick={() => locate(b.entity_id)}>{surface ? `${surface.title ?? surface.name}` : t('aiworkspace.data.canvas')} · {b.view_type ?? t('aiworkspace.data.type.cell')} {b.title ? t('aiworkspace.data.relations.quotedTitle', { title: b.title }) : ''}</button>
      })}
      {r.produced.length > 0 && (
        <>
          <div className="aiws-panel-title">{t('aiworkspace.data.relations.produced')} <span className="aiws-muted">{r.produced.length}</span></div>
          {r.produced.map((p) => <button key={p.entity_id} type="button" className="aiws-link" onClick={() => ui.openEntity(p.entity_id)}>{label(p, p.entity_id)}</button>)}
        </>
      )}
    </div>
  )
}
