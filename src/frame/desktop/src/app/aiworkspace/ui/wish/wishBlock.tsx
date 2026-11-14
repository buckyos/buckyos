/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* The wish Block: a static card (prompt, executor, freshness) on the canvas. On a free canvas the full flow
 * opens in the right panel and the card stays a card, marked "open" (标准对象的交互改进 §4.4, R6). Its actions — run
 * (or cancel the run in progress) and open — are on its toolbar, in its menu and on a phone in its context window;
 * hovering it shows the same "run" and "open" (UI 框架 PRD §10.1). A flow page and the data-source detail still show
 * the flow inside the Block. */

import { createContext, useCallback, useContext, useSyncExternalStore } from 'react'
import { CircleStop, PanelRight, Play, Sparkles } from 'lucide-react'
import { randomId } from '../../api/ids'
import type { ReadOk } from '../../api/session'
import { SYSTEM_IDS, type Json, type WishPayloadRead } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useLoad, useStore, useVersion } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import type { WorkspaceStore } from '../../state/store'
import { requestIntent } from '../blocks/editorToolbar'
import { cellOp, createOp } from '../blocks/ops'
import { blockRegistry, type BlockAction, type BlockDefinition, type RenderContext } from '../blocks/registry'
import { runMacro } from '../framework/macros'
import { FreshnessBadge } from '../sources/FreshnessBadge'
import { macroRunnable } from './macro'
import { WishPanel } from './WishPanel'
import { isActive } from './WishService'
import { MOCK_WISH_DEF_ID, mockWishDefOp } from './mockWishDef'

/** The wish Block whose flow is open in the right panel (its card shows that). */
export const OpenWishContext = createContext<string | null>(null)

const EXECUTOR_LABEL: Record<string, string> = { mock: 'aiworkspace.wish.executorShort.mock', xllm: 'aiworkspace.wish.executorShort.xllm' }

/** The executor's short name (an unknown one is shown as its id). */
function executorLabel(executor: string | null | undefined, t: (key: string) => string): string {
  const key = EXECUTOR_LABEL[executor ?? '']
  return key ? t(key) : executor ?? ''
}

function WishStatic(context: RenderContext) {
  const store = useStore()
  const { t } = useAiwsText()
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
        <b>{context.payload.title ?? payload?.title ?? t('aiworkspace.wish.blockTitle')}</b>
        <span className="aiws-chip aiws-chip-derived">{executorLabel(payload?.executor, t)}</span>
        <FreshnessBadge entityId={id} />
      </div>
      <div className="aiws-wish-card-prompt">{payload?.prompt ?? '…'}</div>
      <div className="aiws-muted">{t('aiworkspace.wish.card.inputs', { count: payload?.inputs?.length ?? 0 })} · {payload?.last_run
        ? t(payload.last_run.checks?.failed ? 'aiworkspace.wish.card.lastRunFailed' : 'aiworkspace.wish.card.lastRun', { count: Object.keys(payload.last_run.result_bindings?.results ?? {}).length || (payload.last_run.produced?.length ?? 0), failed: payload.last_run.checks?.failed ?? 0 })
        : t('aiworkspace.wish.card.notRun')}</div>
      {(open || running) && <div className="aiws-wish-card-state" data-testid="aiws-wish-card-state">{running ? t('aiworkspace.wish.card.running') : t('aiworkspace.wish.card.editingInPanel')}</div>}
    </div>
  )
}

function WishEditorView(context: RenderContext) {
  if (!context.source) return null
  return <WishPanel wishId={context.source.entity_id} cellId={context.cell.entity_id} readOnly={context.readOnlyReason !== null} compact />
}

const wishOf = (context: RenderContext) => context.source?.entity_id ?? ''
const running = (context: RenderContext, store: WorkspaceStore) => isActive(store.wish.run(wishOf(context)))

/** Why "run" cannot start here: in edit mode the wish's panel takes the next step (it says what is missing); elsewhere
 * (view mode, a phone's free canvas) only a macro that can run now runs, as the "宏" entry runs it. */
function runReason(context: RenderContext, store: WorkspaceStore): string | false {
  if (context.mode === 'edit') return false
  const wish = context.source
  if (!wish?.capabilities.includes('update')) return aiwsT('aiworkspace.blocks.noPermission')
  return macroRunnable(wish, store.freshness.get(wish.entity_id)) ? false : aiwsT('aiworkspace.blockprops.wishNotReady')
}

/** "Run" (the wish panel's run): in edit mode the panel opens and performs the wish's next action there
 * (`WishService.next`: analyze, code, run, or apply); elsewhere its macro runs as the "宏" entry runs it
 * (`WishService.runMacro`; what becomes of it is in the work log). */
function runWish(context: RenderContext, store: WorkspaceStore) {
  if (context.mode === 'edit') {
    requestIntent(context.cell.entity_id, 'start')
    context.activateEditor()
    return
  }
  const wish = context.source
  if (!wish) return
  runMacro(store, { wishId: wish.entity_id, title: wish.title ?? wish.name ?? wish.entity_id, cellId: context.cell.entity_id, onSurface: true, busy: false }, context.openEntity)
}

const wishActions: BlockAction[] = [
  { id: 'wish-run', icon: Play, primary: true, ai: true, get label() { return aiwsT('aiworkspace.wish.run') }, modes: ['edit', 'view'],
    labelFor: (context, store) => aiwsT(store.outline.all().some((e) => !e.deleted && e.derived?.wish_id === wishOf(context)) ? 'aiworkspace.wish.runAgain' : 'aiworkspace.wish.run'),
    when: (context, store) => Boolean(context.source) && !running(context, store), disabled: runReason, run: runWish },
  { id: 'wish-cancel', icon: CircleStop, primary: true, get label() { return aiwsT('aiworkspace.wish.cancelRun') }, modes: ['edit', 'view'],
    when: running, run: (context, store) => { const run = store.wish.run(wishOf(context)); if (run) void store.wish.cancel(run) } },
  { id: 'wish-open', icon: PanelRight, primary: true, get label() { return aiwsT('aiworkspace.wish.open') }, modes: ['edit'], run: (context) => context.activateEditor(), key: 'Enter' },
]

export const wishBlock: BlockDefinition = {
  type: 'wish', version: 1, get title() { return aiwsT('aiworkspace.wish.blockTitle') }, accepts: ['buckyos.wish'], allowNoSource: false,
  defaultSize: { w: 420, h: 300 }, cost: { editor: true, html: false },
  Static: WishStatic, Editor: WishEditorView,
  catalog: { group: 'ai', get description() { return aiwsT('aiworkspace.wish.blockDescription') }, needs: 'none', editAfterInsert: true, standard: true, toolPicker: 9 },
  actions: wishActions,
  // the hover buttons are its "run" (or "cancel") and "open" actions
  hover: (context, store) => {
    const wishId = context.source?.entity_id
    if (!wishId) return []
    const run = running(context, store) ? wishActions[1] : wishActions[0]
    const shown = run === wishActions[0] && runReason(context, store) ? null : run
    return [
      { id: 'name', kind: 'label', at: 'top-left-out', icon: Sparkles, modes: ['edit', 'view'], freshness: wishId,
        label: `${context.payload.title ?? context.source?.title ?? aiwsT('aiworkspace.wish.blockTitle')} · ${executorLabel(context.source?.executor, aiwsT)}` },
      ...(shown ? [{ id: shown.id, kind: 'button' as const, at: 'top-right-in' as const, icon: shown.icon, label: shown.labelFor?.(context, store) ?? shown.label, modes: shown.modes, action: shown.id }] : []),
      { id: 'open', kind: 'button', at: 'bottom-right-in', icon: PanelRight, label: aiwsT('aiworkspace.wish.open'), action: 'wish-open' },
    ]
  },
  create: (args) => {
    const ops = []
    // the Mock executor's definition entity is created with the first wish of a Workspace (D9)
    if (!args.store.outline.get(MOCK_WISH_DEF_ID)) ops.push(mockWishDefOp(args.store.core.order_key_between(args.store.outline.childrenOf('data').filter((e) => !SYSTEM_IDS.has(e.entity_id)).at(-1)?.order_key ?? undefined, undefined)))
    if (args.existingSourceId) return [...ops, cellOp(args, 'wish', args.existingSourceId)]
    const title = args.title || aiwsT('aiworkspace.wish.blockTitle')
    const payload: Record<string, Json> = {
      title, prompt: args.title || '', executor: 'xllm', output_mode: 'overwrite',
      output: { container_id: args.contentFolderId, surface_id: args.surfaceId, name: aiwsT('aiworkspace.wish.resultsFolder', { title }) },
    }
    return [...ops, createOp(args.dataId, 'buckyos.wish', args.contentFolderId, args.dataOrderKey, payload, args.title || `wish-${randomId().slice(0, 6)}`), cellOp(args, 'wish', args.dataId)]
  },
}

export function registerWishBlock() {
  blockRegistry.register(wishBlock)
}
