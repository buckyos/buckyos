/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* The `html` Block (phase two §10.4): a Block whose definition entity carries HTML / CSS / JS. In
 * edit and view mode it shows its last static snapshot (or a placeholder) and runs only after
 * explicit activation; while mounted, the extension talks to the host through `window.aiws`.
 * Crashes and silence fall back to the snapshot; other Blocks are unaffected. */

import { useEffect, useRef, useState } from 'react'
import type { Json } from '../../api/types'
import { useStore } from '../../state/hooks'
import { AssetBlobImage } from './AssetBlobImage'
import { blockRegistry, type BlockDefinition, type RenderContext } from '../blocks/registry'
import { cellOp } from '../blocks/ops'
import { makeBridge } from './bridge'
import { HtmlRuntime } from './htmlRuntime'

function Snapshot(context: RenderContext & { label?: string }) {
  const snapshot = context.payload.config?.snapshot
  return (
    <div className="aiws-html-static" data-testid={`aiws-html-static-${context.cell.entity_id}`}>
      {snapshot?.object_id
        ? <AssetBlobImage objectId={snapshot.object_id} mediaType={snapshot.media_type} alt={context.payload.title ?? 'snapshot'} />
        : <div className="aiws-html-placeholder">{context.label ?? 'HTML 扩展'}<div className="aiws-muted">尚无静态快照；在编辑模式中激活后运行</div></div>}
    </div>
  )
}

function HtmlStatic(context: RenderContext) {
  return <Snapshot {...context} label={context.documentDefinition?.title ?? context.payload.title ?? 'HTML 扩展'} />
}

/** Mounted after activation: the real runtime. */
function HtmlActive(context: RenderContext) {
  const store = useStore()
  const hostRef = useRef<HTMLDivElement>(null)
  const [failure, setFailure] = useState<string | null>(null)
  const [attempt, setAttempt] = useState(0)
  const source = context.documentDefinition
  const html = source?.kind === 'html' ? source.html : undefined
  const htmlKey = html ? JSON.stringify(html) : ''
  const cellId = context.cell.entity_id
  const mode = context.mode
  const deactivate = context.deactivateEditor
  useEffect(() => {
    const host = hostRef.current
    if (!host || !htmlKey) return
    const src = JSON.parse(htmlKey) as { html: string; css?: string; js?: string }
    const cell = store.outline.get(cellId)
    if (!cell) return
    const runtime = new HtmlRuntime(src, makeBridge(store, { cell, payload: context.payload, keyRevs: context.keyRevs, source: context.source, mode }))
    let cancelled = false
    runtime.onCrash = (message) => { if (!cancelled) { runtime.dispose(); setFailure(message) } }
    runtime.mount(host).catch((error: unknown) => { if (!cancelled) setFailure(error instanceof Error ? error.message : String(error)) })
    return () => { cancelled = true; runtime.dispose() }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store, htmlKey, cellId, mode, attempt])
  if (!html) return <div className="aiws-warning">HTML 定义不可用。</div>
  if (failure) {
    return (
      <div className="aiws-block-fallback" data-testid={`aiws-html-failed-${context.cell.entity_id}`} data-reason="html_failed">
        <div className="aiws-error" role="alert">HTML 扩展出错或无响应：{failure}</div>
        <Snapshot {...context} />
        <button type="button" className="aiws-link" onClick={() => { setFailure(null); setAttempt((n) => n + 1) }}>重新运行</button>
        <button type="button" className="aiws-link" onClick={deactivate}>回到静态显示</button>
      </div>
    )
  }
  return (
    <div className="aiws-html-active" data-testid={`aiws-html-active-${context.cell.entity_id}`}>
      <div ref={hostRef} className="aiws-html-host" />
      <div className="aiws-html-bar"><span className="aiws-muted">HTML 扩展已挂载 · {source?.title ?? context.payload.def_ref?.entity_id}</span><button type="button" className="aiws-link" onClick={deactivate}>停止</button></div>
    </div>
  )
}

export const htmlBlock: BlockDefinition = {
  type: 'html', version: 1, definitionKind: 'html', title: 'HTML 扩展', accepts: ['buckyos.table-source', 'buckyos.richtext', 'buckyos.record', 'buckyos.asset-ref', 'buckyos.wish', 'buckyos.annotation'], allowNoSource: true,
  defaultSize: { w: 480, h: 320 }, cost: { editor: true, html: true },
  Static: HtmlStatic, Editor: HtmlActive,
  actions: [
    { id: 'run', label: '运行扩展', modes: ['edit'], run: (context) => context.activateEditor(), key: 'Enter' },
    { id: 'open-def', label: '打开定义', modes: ['edit', 'view'], when: (context) => Boolean(context.payload.def_ref), run: (context) => { if (context.payload.def_ref) context.openEntity(context.payload.def_ref.entity_id) } },
  ],
  create: (args) => [cellOp(args, 'html', args.existingSourceId, { def_ref: { entity_id: String(args.config?.def_id ?? '') } as unknown as Json })],
}

export function registerHtmlBlock() {
  blockRegistry.register(htmlBlock)
}
