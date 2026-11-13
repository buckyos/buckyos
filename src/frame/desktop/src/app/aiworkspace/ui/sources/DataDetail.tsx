/* The middle column of the data-source view (phase two §6.1): the data itself, editable without a
 * Block, through the same editors the Blocks activate; plus the relations / dependencies tab. */

import { useShell } from '../shell/shellContext'
import { useCallback, useState } from 'react'
import type { ReadOk } from '../../api/session'
import type { BlockDefRead, EntityEnvelope, Reference } from '../../api/types'
import { useDirectReadOnly, useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { AssetCell, NoteEditor, RecordCell, RichTextCell, TableEditor } from '../blocks/editors'
import { WishPanel } from '../wish/WishPanel'
import { RelationsPanel } from './RelationsPanel'
import { PublicationSection, SourceSection } from './SourcePanel'
import { TYPE_LABEL, entityLabel } from './dataOps'
import { FreshnessBadge } from './FreshnessBadge'
import { DirectoryDetail } from './DirectoryDetail'

export function DataDetail({ entity }: { entity: EntityEnvelope }) {
  const { t, tNode } = useAiwsText()
  const store = useStore()
  const ui = useWorkspaceUi()
  const [tab, setTab] = useState<'content' | 'relations'>('content')
  const readOnly = useDirectReadOnly()
  const blocks = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.source_id === entity.entity_id && !e.deleted)
  // data a canvas owns (内核架构改进 D05): pinning ends the ownership and moves it to the data tree
  const ownData = entity.type_id !== 'buckyos.container' && store.outline.ancestors(entity.entity_id).includes('canvas-content')
  const pin = async () => {
    const key = store.core.order_key_between(store.outline.childrenOf('data').at(-1)?.order_key ?? undefined, undefined)
    await store.submit({ editId: `pin:${entity.entity_id}`, label: t('aiworkspace.data.detail.pinLabel', { name: entityLabel(entity) }), operations: [
      { op: 'data.pin', entity_id: entity.entity_id },
      { op: 'tree.move', entity_id: entity.entity_id, new_parent_id: 'data', order_key: key },
    ] })
  }
  return (
    <div className="aiws-detail" data-testid={`aiws-detail-${entity.entity_id}`} data-type={entity.type_id}>
      <div className="aiws-detail-head">
        <span className="aiws-outline-type">{TYPE_LABEL[entity.type_id] ?? entity.type_id}</span>
        <b>{entityLabel(entity)}</b>
        {entity.derived && <FreshnessBadge entityId={entity.entity_id} />}
        <span className="aiws-grow" />
        <button type="button" role="tab" aria-selected={tab === 'content'} data-testid="aiws-detail-tab-content" onClick={() => setTab('content')}>{t('aiworkspace.data.detail.content')}</button>
        <button type="button" role="tab" aria-selected={tab === 'relations'} data-testid="aiws-detail-tab-relations" onClick={() => setTab('relations')}>{t('aiworkspace.data.detail.relations')}</button>
      </div>
      {tab === 'relations' ? <RelationsPanel entityId={entity.entity_id} /> : (
        <div className="aiws-detail-body">
          {ownData && (
            <div className="aiws-inline-form" data-testid="aiws-own-data">
              <span className="aiws-muted">{t('aiworkspace.data.detail.ownData')}</span>
              {entity.capabilities.includes('structure') && !readOnly && <button type="button" data-testid="aiws-data-pin" onClick={() => { void pin() }}>{t('aiworkspace.data.detail.pin')}</button>}
            </div>
          )}
          <SourceSection entity={entity} />
          <PublicationSection entity={entity} />
          {blocks.length > 0 && (
            <div className="aiws-muted aiws-detail-blocks" data-testid="aiws-detail-blocks">{tNode('aiworkspace.data.detail.onCanvas', {
              blocks: blocks.map((b) => {
                const surface = store.outline.ancestors(b.entity_id).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')
                return <button key={b.entity_id} type="button" className="aiws-link" data-testid={`aiws-locate-${b.entity_id}`} onClick={() => ui.openEntity(b.entity_id)}>{surface?.title ?? surface?.name ?? t('aiworkspace.data.canvas')} · {b.view_type}</button>
              }),
            })}</div>
          )}
          <DataContent entity={entity} />
        </div>
      )}
    </div>
  )
}

function DataContent({ entity }: { entity: EntityEnvelope }) {
  const ui = useWorkspaceUi()
  const readOnly = useDirectReadOnly()
  const renderEmbed = useCallback((reference: Reference) => ui.renderCell(reference.entity_id, 1), [ui])
  if (entity.degraded) return <RawDetail entity={entity} />
  switch (entity.type_id) {
    case 'buckyos.table-source': return <TableEditor sourceId={entity.entity_id} source={entity} readOnly={readOnly || Boolean(entity.source)} />
    case 'buckyos.richtext': return <RichTextCell entity={entity} entityId={entity.entity_id} renderEmbed={renderEmbed} />
    case 'buckyos.record': return <RecordCell entityId={entity.entity_id} entity={entity} readOnly={readOnly} />
    case 'buckyos.asset-ref': return <AssetCell entityId={entity.entity_id} fit="contain" readOnly={readOnly || Boolean(entity.source)} />
    case 'buckyos.annotation': return <NoteEditor entityId={entity.entity_id} entity={entity} readOnly={readOnly} />
    case 'buckyos.wish': return <WishPanel wishId={entity.entity_id} readOnly={readOnly} />
    case 'buckyos.block-def': return <BlockDefDetail entity={entity} />
    case 'buckyos.container': return <FolderDetail entity={entity} />
    case 'buckyos.dir': return <DirectoryDetail entity={entity} readOnly={readOnly} />
    default: return <RawDetail entity={entity} />
  }
}

function FolderDetail({ entity }: { entity: EntityEnvelope }) {
  const { t } = useAiwsText()
  const store = useStore()
  const ui = useWorkspaceUi()
  const children = store.outline.childrenOf(entity.entity_id)
  const surface = entity.surface_id ? store.outline.get(entity.surface_id) : undefined
  return (
    <div className="aiws-folder-detail">
      {entity.system === 'surface_content' && surface && <div className="aiws-muted">{t('aiworkspace.data.detail.surfaceContent', { name: entityLabel(surface) })}</div>}
      {entity.system === 'canvas_content' && <div className="aiws-muted">{t('aiworkspace.data.detail.canvasContent')}</div>}
      {entity.system === 'shows' && <div className="aiws-muted">{t('aiworkspace.data.detail.shows')}</div>}
      <ul>{children.map((c) => <li key={c.entity_id}><button type="button" className="aiws-link" onClick={() => ui.openEntity(c.entity_id)}>{entityLabel(c)}</button> <span className="aiws-muted">{TYPE_LABEL[c.type_id] ?? c.type_id}</span></li>)}</ul>
      {children.length === 0 && <div className="aiws-muted">{t('aiworkspace.data.detail.emptyFolder')}</div>}
    </div>
  )
}

function BlockDefDetail({ entity }: { entity: EntityEnvelope }) {
  const { t, tNode, list, dateTime } = useAiwsText()
  const store = useStore()
  const shell = useShell()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<BlockDefRead>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<BlockDefRead>>(load, version)
  if (!read.data) return <div className="aiws-muted">{read.error ?? t('aiworkspace.data.detail.defLoading')}</div>
  const payload = read.data.content.payload
  const build = (read.data.derived as { build?: { built_at: string } } | null | undefined)?.build
  const users = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.def_id === entity.entity_id && !e.deleted)
  return (
    <div className="aiws-def-detail" data-testid="aiws-def-detail">
      <div>{tNode(payload.accepts ? 'aiworkspace.data.detail.defSummaryAccepts' : 'aiworkspace.data.detail.defSummary', {
        id: <code>{payload.def_id}</code>, version: payload.version ?? 1,
        kind: payload.kind === 'html' ? t('aiworkspace.data.detail.defHtml') : t('aiworkspace.data.detail.defDeclarative'), accepts: list(payload.accepts ?? []),
      })}</div>
      {payload.description && <div className="aiws-muted">{payload.description}</div>}
      {payload.ident && <div className="aiws-muted" data-testid="aiws-def-ident">{t('aiworkspace.data.detail.defIdent', { ident: `${payload.ident.author_did}#${payload.ident.name}` })}</div>}
      {payload.provenance?.pkg_id && <div className="aiws-muted">{t('aiworkspace.data.detail.defProvenance', { pkg: `${payload.provenance.pkg_id} ${payload.provenance.version ?? ''}` })}</div>}
      {build && <div className="aiws-muted">{t('aiworkspace.data.detail.defBuilt', { at: dateTime(build.built_at) })}</div>}
      <div className="aiws-muted">{t('aiworkspace.data.detail.defUsers', { count: users.length })}</div>
      <button type="button" className="aiws-link" data-testid="aiws-def-manage" onClick={() => shell.openDialog({ kind: 'extensions', defId: entity.entity_id })}>{t('aiworkspace.ext.manage')}</button>
      {payload.kind === 'html' && <div className="aiws-warning">{t('aiworkspace.data.detail.defHtmlRisk')}</div>}
      <details><summary>{t('aiworkspace.data.detail.defShow')}</summary><pre className="aiws-raw">{JSON.stringify(payload.kind === 'html' ? { ...payload, html: { ...payload.html, js: t('aiworkspace.data.detail.defChars', { count: payload.html?.js?.length ?? 0 }) } } : payload, null, 2)}</pre></details>
    </div>
  )
}

function RawDetail({ entity }: { entity: EntityEnvelope }) {
  const { t } = useAiwsText()
  const store = useStore()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<unknown>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<unknown>>(load, version)
  return (
    <details open data-testid="aiws-raw-detail">
      <summary>{entity.degraded ? t('aiworkspace.data.detail.raw', { reason: entity.degraded }) : t('aiworkspace.data.detail.rawContent')}</summary>
      {!entity.degraded && <p className="aiws-muted">{t('aiworkspace.data.detail.noEditor', { type: TYPE_LABEL[entity.type_id] ?? entity.type_id })}</p>}
      {read.error && <div className="aiws-error" role="alert">{read.error}</div>}
      {read.data ? <pre className="aiws-raw">{JSON.stringify(read.data.content, null, 2)}</pre> : !read.error && <div className="aiws-muted">{t('aiworkspace.blocks.loading')}</div>}
    </details>
  )
}
