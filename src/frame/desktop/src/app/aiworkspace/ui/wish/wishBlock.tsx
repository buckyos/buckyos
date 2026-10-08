/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* The wish Block: a static card (prompt, executor, freshness) on the canvas. On a free canvas the full flow
 * opens in the right panel and the card stays a card, marked "open" (标准对象的交互改进 §4.4, R6); hovering
 * it offers "run" and "open", and its toolbar runs, cancels and opens it. A flow page and the data-source
 * detail still show the flow inside the Block. */

import { createContext, useCallback, useContext, useSyncExternalStore } from 'react'
import { CircleStop, Database, PanelRight, Play, Sparkles } from 'lucide-react'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import { SYSTEM_IDS, type Json, type WishPayloadRead } from '../../api/types'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { requestIntent } from '../blocks/editorToolbar'
import { cellOp, createOp } from '../blocks/ops'
import { blockRegistry, type BlockDefinition, type RenderContext, type ToolbarItem } from '../blocks/registry'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { WishPanel } from './WishPanel'
import { isActive } from './WishService'
import { MOCK_WISH_DEF_ID, mockWishDefOp } from './mockWishDef'

/** The wish Block whose flow is open in the right panel (its card shows that). */
export const OpenWishContext = createContext<string | null>(null)

const EXECUTOR_LABEL: Record<string, string> = { mock: '模拟', xllm: '模型' }

function WishStatic(context: RenderContext) {
  const store = useStore()
  const id = context.source?.entity_id ?? ''
  const version = useVersion(`e:${id}`)
  const load = useCallback(() => store.readBatched<WishPayloadRead>(id), [store, id])
  const read = useLoad<ReadOk<WishPayloadRead>>(load, version)
  useSyncExternalStore(store.wish.subscribe, store.wish.snapshot)
  const open = useContext(OpenWishContext) === context.cell.entity_id
  const payload = read.data?.content.payload
  const running = isActive(store.wish.run(id)) || Boolean(store.wish.busy.get(id))
  return (
    <div className="aiws-wish-card aiws-block-static" data-testid={`aiws-wish-card-${id}`} data-open={open ? 'true' : 'false'}>
      <div className="aiws-wish-head">
        <b>{context.payload.title ?? payload?.title ?? '许愿格'}</b>
        <span className="aiws-chip aiws-chip-derived">{EXECUTOR_LABEL[payload?.executor ?? ''] ?? payload?.executor ?? ''}</span>
        <FreshnessBadge entityId={id} />
      </div>
      <div className="aiws-wish-card-prompt">{payload?.prompt ?? '…'}</div>
      <div className="aiws-muted">{payload?.inputs?.length ?? 0} 个输入 · {payload?.last_run ? `上次运行 ${Object.keys(payload.last_run.result_bindings?.results ?? {}).length || (payload.last_run.produced?.length ?? 0)} 项结果${payload.last_run.checks?.failed ? `，${payload.last_run.checks.failed} 项检查未通过` : ''}` : '尚未运行'}</div>
      {(open || running) && <div className="aiws-wish-card-state" data-testid="aiws-wish-card-state">{running ? '运行中…' : '正在右侧面板中编辑'}</div>}
    </div>
  )
}

function WishEditorView(context: RenderContext) {
  if (!context.source) return null
  return <WishPanel wishId={context.source.entity_id} cellId={context.cell.entity_id} readOnly={context.readOnlyReason !== null} compact />
}

/** Open the wish (the right panel on a free canvas) and start its next step: analysis, or execution once analysed. */
function runWish(context: RenderContext) {
  requestIntent(context.cell.entity_id, 'start')
  context.activateEditor()
}

/** Run, or cancel the run in progress. */
function runOrCancel(context: RenderContext, store: WorkspaceStore): ToolbarItem {
  const wishId = context.source?.entity_id ?? ''
  const run = store.wish.run(wishId)
  if (isActive(run) && run) return { kind: 'button', id: 'wish-cancel', icon: CircleStop, label: '取消运行', run: () => { void store.wish.cancel(run) } }
  const ran = store.outline.all().some((e) => !e.deleted && e.derived?.wish_id === wishId)
  return { kind: 'button', id: 'wish-run', icon: Play, label: ran ? '重新运行' : '运行', ai: true, disabled: context.mode === 'edit' ? false : '查看模式不能运行', run: () => runWish(context) }
}

export const wishBlock: BlockDefinition = {
  type: 'wish', version: 1, title: '许愿格', accepts: ['buckyos.wish'], allowNoSource: false,
  defaultSize: { w: 420, h: 300 }, cost: { editor: true, html: false },
  Static: WishStatic, Editor: WishEditorView,
  catalog: { group: 'ai', description: '描述要完成的任务，AI 分析当前画布和选中的数据后给出候选结果；预览、反馈后再应用，整次应用可撤销。', needs: 'none', editAfterInsert: true, standard: true },
  actions: [
    { id: 'open', label: '打开许愿格', modes: ['edit'], run: (context) => context.activateEditor(), key: 'Enter' },
    { id: 'detail', label: '在数据源中查看', modes: ['edit', 'view'], run: (context) => { if (context.source) context.openEntity(context.source.entity_id) } },
  ],
  hover: (context, store) => {
    const wishId = context.source?.entity_id
    if (!wishId) return []
    const run = runOrCancel(context, store)
    return [
      { id: 'name', kind: 'label', at: 'top-left-out', icon: Sparkles, modes: ['edit', 'view'], freshness: wishId,
        label: `${context.payload.title ?? context.source?.title ?? '许愿格'} · ${EXECUTOR_LABEL[context.source?.executor ?? ''] ?? context.source?.executor ?? ''}` },
      ...(run.kind === 'button' && !run.disabled ? [{ id: run.id, kind: 'button' as const, at: 'top-right-in' as const, icon: run.icon, label: run.label, run: run.run }] : []),
      { id: 'open', kind: 'button', at: 'bottom-right-in', icon: PanelRight, label: '打开许愿格', action: 'open' },
    ]
  },
  toolbar: (context, store) => [
    runOrCancel(context, store),
    { kind: 'button', id: 'wish-open', icon: PanelRight, label: '打开许愿格', key: 'Enter', disabled: context.mode === 'edit' ? false : '查看模式不能编辑', run: () => context.activateEditor() },
    { kind: 'button', id: 'wish-detail', icon: Database, label: '在数据源中查看', run: () => { if (context.source) context.openEntity(context.source.entity_id) } },
  ],
  create: (args) => {
    const ops = []
    // the Mock executor's definition entity is created with the first wish of a Workspace (D9)
    if (!args.store.outline.get(MOCK_WISH_DEF_ID)) ops.push(mockWishDefOp(args.store.core.order_key_between(args.store.outline.childrenOf('data').filter((e) => !SYSTEM_IDS.has(e.entity_id)).at(-1)?.order_key ?? undefined, undefined)))
    if (args.existingSourceId) return [...ops, cellOp(args, 'wish', args.existingSourceId)]
    const payload: Record<string, Json> = {
      title: args.title || '许愿格', prompt: args.title || '', executor: 'xllm', output_mode: 'overwrite',
      output: { container_id: args.contentFolderId, surface_id: args.surfaceId, name: `${args.title || '许愿格'}的结果` },
    }
    return [...ops, createOp(args.dataId, 'buckyos.wish', args.contentFolderId, args.dataOrderKey, payload, args.title || `wish-${randomId().slice(0, 6)}`), cellOp(args, 'wish', args.dataId)]
  },
}

export function registerWishBlock() {
  blockRegistry.register(wishBlock)
}
