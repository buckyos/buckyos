/* Relations and dependencies of one entity (phase two §6.2): what it references, who uses it,
 * what generated it and what it generated, with the freshness the core computed. Unreadable
 * referrers are one line, never names or counts. "Locate on canvas" lists the Blocks that show it. */

import { useCallback } from 'react'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import type { RelationsInfo } from '../../api/types'
import { FRESHNESS_LABEL, freshnessDetail } from './freshnessText'
import { TYPE_LABEL } from './dataOps'

const KIND_LABEL: Record<string, string> = { bind: '展现绑定', embed: '富文本嵌入', value: '对象引用', body: '记录正文', anchor: '标注锚点', input: '许愿格输入', derived: '生成依赖', produced: '生成自', def: '使用 Block 定义', asset: '资产内容', connector_endpoint: '连接线端点', show_target: '演讲路径步骤', show_surface: 'Viewport 所在画布' }

export function RelationsPanel({ entityId }: { entityId: string }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const version = useVersion('any')
  const load = useCallback(() => store.session.relations(entityId), [store, entityId])
  const read = useLoad<RelationsInfo>(load, version)
  if (read.error && !read.data) return <div className="aiws-error" role="alert">无法读取引用关系：{read.error}</div>
  if (!read.data) return <div className="aiws-muted">载入引用关系…</div>
  const r = read.data
  const label = (b: { title?: string | null; name: string | null; entity_id: string } | undefined, id: string) => b?.title ?? b?.name ?? id
  const locate = (blockId: string) => ui.openEntity(blockId)
  const freshness = r.freshness
  return (
    <div className="aiws-relations" data-testid={`aiws-relations-${entityId}`}>
      {freshness && freshness.status !== 'none' && (
        <div className={`aiws-relations-freshness${freshness.status === 'current' ? '' : ' aiws-warning'}`} data-testid="aiws-relations-freshness" data-status={freshness.status}>
          <b>{FRESHNESS_LABEL[freshness.status]}</b> · {freshnessDetail(freshness)}
          {freshness.manual_modified && <div>人工已修改：当前内容版本 {freshness.content_rev} ≠ 生成版本 {freshness.generated_rev}</div>}
          {freshness.wish_id && <div className="aiws-muted">生成自许愿格 <button type="button" className="aiws-link" onClick={() => ui.openEntity(freshness.wish_id ?? '')}>{label(store.outline.get(freshness.wish_id ?? ''), freshness.wish_id ?? '')}</button> · 运行 {freshness.run_id} · {freshness.executor}{freshness.simulated ? '（模拟）' : ''}</div>}
          {(freshness.upstream ?? []).length > 0 && <div>间接依赖已过期：{freshness.upstream?.map((u) => <button key={u.entity_id} type="button" className="aiws-link" onClick={() => ui.openEntity(u.entity_id)}>{label(store.outline.get(u.entity_id), u.entity_id)}（{FRESHNESS_LABEL[u.status]}）</button>)}</div>}
        </div>
      )}
      <div className="aiws-panel-title">我引用谁 <span className="aiws-muted">{r.outgoing.length}</span></div>
      {r.outgoing.length === 0 && <div className="aiws-muted">没有引用其他对象。</div>}
      <ul className="aiws-relation-list">
        {r.outgoing.map((line, i) => (
          <li key={i} data-testid="aiws-relation-out" data-kind={line.kind}>
            <span className="aiws-outline-type">{KIND_LABEL[line.kind] ?? line.kind}</span>
            {line.external ? <span>{line.workspace_id} / {line.entity_id}（外部）</span>
              : line.object_id ? <span className="aiws-muted">对象 {line.object_id.slice(0, 20)}…</span>
                : line.readable === false ? <span className="aiws-muted">（无权查看的对象）</span>
                  : line.missing ? <span className="aiws-error">{line.entity_id}（引用不可用）</span>
                    : <button type="button" className="aiws-link" onClick={() => ui.openEntity(line.entity_id)}>{label(line.target, line.entity_id)}{line.target?.deleted ? '（已删除）' : ''}</button>}
          </li>
        ))}
      </ul>
      <div className="aiws-panel-title">谁引用我 <span className="aiws-muted">{r.incoming.length}</span></div>
      {r.incoming.length === 0 && !r.hidden_incoming && <div className="aiws-muted">当前工作区中没有我有权查看的引用。</div>}
      <ul className="aiws-relation-list">
        {r.incoming.map((line, i) => (
          <li key={i} data-testid="aiws-relation-in" data-kind={line.kind}>
            <span className="aiws-outline-type">{KIND_LABEL[line.kind] ?? line.kind}</span>
            <button type="button" className="aiws-link" onClick={() => (line.source?.type_id === 'buckyos.cell' ? locate(line.entity_id) : ui.openEntity(line.entity_id))}>{label(line.source, line.entity_id)}</button>
            <span className="aiws-muted"> {TYPE_LABEL[line.source?.type_id ?? ''] ?? ''}{line.blocks_delete ? ' · 阻止删除' : ''}</span>
          </li>
        ))}
      </ul>
      {r.hidden_incoming && <div className="aiws-muted" data-testid="aiws-relation-hidden">存在无权查看的引用。</div>}
      {r.blocks.length > 0 && (
        <div className="aiws-panel-title">在画布中定位 <span className="aiws-muted">{r.blocks.length} 个 Block</span></div>
      )}
      {r.blocks.map((b) => {
        const surface = store.outline.ancestors(b.entity_id).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')
        return <button key={b.entity_id} type="button" className="aiws-link" data-testid={`aiws-locate-${b.entity_id}`} onClick={() => locate(b.entity_id)}>{surface ? `${surface.title ?? surface.name}` : '画布'} · {b.view_type ?? 'Block'} {b.title ? `「${b.title}」` : ''}</button>
      })}
      {r.produced.length > 0 && (
        <>
          <div className="aiws-panel-title">生成的结果 <span className="aiws-muted">{r.produced.length}</span></div>
          {r.produced.map((p) => <button key={p.entity_id} type="button" className="aiws-link" onClick={() => ui.openEntity(p.entity_id)}>{label(p, p.entity_id)}</button>)}
        </>
      )}
    </div>
  )
}
