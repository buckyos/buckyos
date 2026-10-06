/* The middle column of the data-source view (phase two §6.1): the data itself, editable without a
 * Block, through the same editors the Blocks activate; plus the relations / dependencies tab. */

import { useCallback, useState } from 'react'
import type { ReadOk } from '../../api/session'
import type { BlockDefRead, EntityEnvelope, Reference } from '../../api/types'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { AssetCell, NoteEditor, RecordCell, RichTextCell, TableEditor } from '../blocks/editors'
import { WishPanel } from '../wish/WishPanel'
import { RelationsPanel } from './RelationsPanel'
import { TYPE_LABEL, entityLabel } from './dataOps'
import { FreshnessBadge } from './FreshnessBadge'

export function DataDetail({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const [tab, setTab] = useState<'content' | 'relations'>('content')
  const readOnly = store.session.status().kind === 'offline' && store.session.mode().kind === 'direct' && store.session.mode().kind === 'direct' && (store.session.mode() as { reason?: string }).reason !== 'not_prepared'
  const renderEmbed = useCallback((reference: Reference) => ui.renderCell(reference.entity_id, 1), [ui])
  const blocks = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.source_id === entity.entity_id && !e.deleted)
  return (
    <div className="aiws-detail" data-testid={`aiws-detail-${entity.entity_id}`} data-type={entity.type_id}>
      <div className="aiws-detail-head">
        <span className="aiws-outline-type">{TYPE_LABEL[entity.type_id] ?? entity.type_id}</span>
        <b>{entityLabel(entity)}</b>
        {entity.derived && <FreshnessBadge entityId={entity.entity_id} />}
        <span className="aiws-grow" />
        <button type="button" role="tab" aria-selected={tab === 'content'} data-testid="aiws-detail-tab-content" onClick={() => setTab('content')}>内容</button>
        <button type="button" role="tab" aria-selected={tab === 'relations'} data-testid="aiws-detail-tab-relations" onClick={() => setTab('relations')}>引用与依赖</button>
      </div>
      {tab === 'relations' ? <RelationsPanel entityId={entity.entity_id} /> : (
        <div className="aiws-detail-body">
          {blocks.length > 0 && (
            <div className="aiws-muted aiws-detail-blocks" data-testid="aiws-detail-blocks">在画布中：{blocks.map((b) => {
              const surface = store.outline.ancestors(b.entity_id).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')
              return <button key={b.entity_id} type="button" className="aiws-link" data-testid={`aiws-locate-${b.entity_id}`} onClick={() => ui.openEntity(b.entity_id)}>{surface?.title ?? surface?.name ?? '画布'} · {b.view_type}</button>
            })}</div>
          )}
          {entity.type_id === 'buckyos.table-source' && <TableEditor sourceId={entity.entity_id} source={entity} readOnly={readOnly} />}
          {entity.type_id === 'buckyos.richtext' && <RichTextCell key={entity.entity_id} entity={entity} entityId={entity.entity_id} renderEmbed={renderEmbed} />}
          {entity.type_id === 'buckyos.record' && <RecordCell entityId={entity.entity_id} entity={entity} readOnly={readOnly} />}
          {entity.type_id === 'buckyos.asset-ref' && <AssetCell entityId={entity.entity_id} fit="contain" readOnly={readOnly} />}
          {entity.type_id === 'buckyos.annotation' && <NoteEditor entityId={entity.entity_id} entity={entity} readOnly={readOnly} />}
          {entity.type_id === 'buckyos.wish' && <WishPanel wishId={entity.entity_id} readOnly={readOnly} />}
          {entity.type_id === 'buckyos.block-def' && <BlockDefDetail entity={entity} />}
          {entity.type_id === 'buckyos.container' && <FolderDetail entity={entity} />}
          {entity.degraded && <RawDetail entity={entity} />}
        </div>
      )}
    </div>
  )
}

function FolderDetail({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const children = store.outline.childrenOf(entity.entity_id)
  const surface = entity.surface_id ? store.outline.get(entity.surface_id) : undefined
  return (
    <div className="aiws-folder-detail">
      {entity.system === 'surface_content' && surface && <div className="aiws-muted">画布「{entityLabel(surface)}」上创建的内容都放在这里；它与画布同时创建、同时删除。</div>}
      {entity.system === 'canvas_content' && <div className="aiws-muted">画布内容区：每张画布一个子文件夹。它不干扰第一层数据源。</div>}
      <ul>{children.map((c) => <li key={c.entity_id}><button type="button" className="aiws-link" onClick={() => ui.openEntity(c.entity_id)}>{entityLabel(c)}</button> <span className="aiws-muted">{TYPE_LABEL[c.type_id] ?? c.type_id}</span></li>)}</ul>
      {children.length === 0 && <div className="aiws-muted">（空文件夹）</div>}
    </div>
  )
}

function BlockDefDetail({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<BlockDefRead>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<BlockDefRead>>(load, version)
  if (!read.data) return <div className="aiws-muted">{read.error ?? '载入定义…'}</div>
  const payload = read.data.content.payload
  const users = store.outline.all().filter((e) => e.type_id === 'buckyos.cell' && e.def_id === entity.entity_id && !e.deleted)
  return (
    <div className="aiws-def-detail" data-testid="aiws-def-detail">
      <div>定义 <code>{payload.def_id}</code> v{payload.version ?? 1} · {payload.kind === 'html' ? 'HTML 扩展' : '声明式'}{payload.accepts ? ` · 接受 ${payload.accepts.join('、')}` : ''}</div>
      {payload.description && <div className="aiws-muted">{payload.description}</div>}
      <div className="aiws-muted">被 {users.length} 个 Block 使用（引用阻止删除）。</div>
      {payload.kind === 'html' && <div className="aiws-warning">HTML 定义以 Owner 的会话同源运行，不设沙盒（D16）；风险由 Owner 管理。</div>}
      <details><summary>查看定义内容</summary><pre className="aiws-raw">{JSON.stringify(payload.kind === 'html' ? { ...payload, html: { ...payload.html, js: `(${payload.html?.js?.length ?? 0} 字符)` } } : payload, null, 2)}</pre></details>
    </div>
  )
}

function RawDetail({ entity }: { entity: EntityEnvelope }) {
  const store = useStore()
  const version = useVersion(`e:${entity.entity_id}`)
  const load = useCallback(() => store.session.read<{ payload: unknown }>(entity.entity_id), [store, entity.entity_id])
  const read = useLoad<ReadOk<{ payload: unknown }>>(load, version)
  return <details open><summary>原始内容（{entity.degraded}，只读）</summary><pre className="aiws-raw">{JSON.stringify(read.data?.content.payload ?? null, null, 2)}</pre></details>
}
