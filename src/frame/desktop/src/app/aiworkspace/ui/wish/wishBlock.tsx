/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* The wish Block: static card (prompt, executor, freshness) on the canvas; the full flow after
 * explicit activation in edit mode or in the data-source detail. */

import { useCallback } from 'react'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import type { Json, WishPayloadRead } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { cellOp, createOp } from '../blocks/ops'
import { blockRegistry, type BlockDefinition, type RenderContext } from '../blocks/registry'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { WishPanel } from './WishPanel'
import { MOCK_WISH_DEF_ID, mockWishDefOp } from './mockWishDef'

function WishStatic(context: RenderContext) {
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(() => store.readBatched<WishPayloadRead>(id), [store, id])
  const read = useLoad<ReadOk<WishPayloadRead>>(load, version)
  const payload = read.data?.content.payload
  return (
    <div className="aiws-wish-card aiws-block-static" data-testid={`aiws-wish-card-${id}`}>
      <div className="aiws-wish-head">
        <b>{context.payload.title ?? payload?.title ?? '许愿格'}</b>
        <span className="aiws-chip aiws-chip-derived">{payload?.executor === 'mock' ? '模拟' : payload?.executor ?? ''}</span>
        <FreshnessBadge entityId={id} />
      </div>
      <div className="aiws-wish-card-prompt">{payload?.prompt ?? '…'}</div>
      <div className="aiws-muted">{payload?.inputs?.length ?? 0} 个输入 · {payload?.last_run ? `上次运行 ${payload.last_run.produced?.length ?? 0} 项结果` : '尚未运行'}{context.mode === 'edit' ? ' · 双击或“打开许愿格”进入流程' : ''}</div>
    </div>
  )
}

function WishEditorView(context: RenderContext) {
  if (!context.source) return null
  return <WishPanel wishId={context.source.entity_id} readOnly={context.readOnlyReason !== null} compact />
}

export const wishBlock: BlockDefinition = {
  type: 'wish', version: 1, title: '许愿格', accepts: ['buckyos.wish'], allowNoSource: false,
  defaultSize: { w: 420, h: 300 }, cost: { editor: true, html: false },
  Static: WishStatic, Editor: WishEditorView,
  actions: [
    { id: 'open', label: '打开许愿格', modes: ['edit'], run: (context) => context.activateEditor(), key: 'Enter' },
    { id: 'detail', label: '在数据源中查看', modes: ['edit', 'view'], run: (context) => { if (context.source) context.openEntity(context.source.entity_id) } },
  ],
  create: (args) => {
    const ops = []
    // the Mock executor's definition entity is created with the first wish of a Workspace (D9)
    if (!args.store.outline.get(MOCK_WISH_DEF_ID)) ops.push(mockWishDefOp(args.store.core.order_key_between(args.store.outline.childrenOf('data').filter((e) => e.entity_id !== 'canvas-content').at(-1)?.order_key ?? undefined, undefined)))
    if (args.existingSourceId) return [...ops, cellOp(args, 'wish', args.existingSourceId)]
    const payload: Record<string, Json> = {
      title: args.title || '许愿格', prompt: args.title || '', executor: 'mock', output_mode: 'overwrite',
      output: { container_id: args.contentFolderId, surface_id: args.surfaceId, name: `${args.title || '许愿格'}的结果` },
    }
    return [...ops, createOp(args.dataId, 'buckyos.wish', args.contentFolderId, args.dataOrderKey, payload, args.title || `wish-${randomId().slice(0, 6)}`), cellOp(args, 'wish', args.dataId)]
  },
}

export function registerWishBlock() {
  blockRegistry.register(wishBlock)
}
