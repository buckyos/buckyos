/* eslint-disable react-refresh/only-export-components -- Block definitions bundle their renderers */
/* The wish Block (许愿格交互升级 §3.1): the wish card, the same component whether the Block is only shown or worked with —
 * there is no editor and no side panel. Small on screen it is the title, the line of state and the run button. Its
 * actions on the toolbar, in the menu and on a phone's context window are the card's run button: run the macro (or stop
 * the run), nothing else (§3.6). Its settings are in the properties panel (§3.7), "显示连线" among them. */

import { CircleStop, Network, Play, Sparkles } from 'lucide-react'
import { needsCodingReasonText } from '../../api/session'
import { randomId } from '../../api/ids'
import type { Json } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { cellOp, createOp } from '../blocks/ops'
import { blockRegistry, type BlockAction, type BlockDefinition, type RenderContext } from '../blocks/registry'
import { runMacro } from '../framework/macros'
import { holdsMacro } from './macro'
import { WishCard } from './WishCard'
import { WishInspector } from './WishInspector'
import { isActive } from './WishService'

/** The card as a Block shows it: in edit mode it can be worked with; a show's stage only where the Block is operable. */
function WishBlockView(context: RenderContext) {
  const id = context.source?.entity_id ?? ''
  const mode = context.mode === 'edit' || (context.mode === 'show' && context.interactive) ? 'edit' : 'view'
  return <WishCard wishId={id} cellId={context.cell.entity_id} mode={mode} readOnly={context.depth > 0 || (context.mode === 'show' && !context.interactive)} />
}

function WishBlockSmall(context: RenderContext) {
  const id = context.source?.entity_id ?? ''
  return <WishCard wishId={id} cellId={context.cell.entity_id} mode={context.mode === 'edit' ? 'edit' : 'view'} readOnly={context.depth > 0} compact />
}

const wishOf = (context: RenderContext) => context.source?.entity_id ?? ''
const running = (context: RenderContext, store: WorkspaceStore) => { const run = store.wish.run(wishOf(context)); return isActive(run) && run?.stage === 'run' }

/** Why "run" cannot run the macro now, as the card's run button says it (§3.6). */
function runReason(context: RenderContext, store: WorkspaceStore): string | false {
  const wish = context.source
  if (!wish) return false
  if (!wish.capabilities.includes('update')) return aiwsT(wish.capabilities.includes('run') ? 'aiworkspace.wish.runOnlyUnavailable' : 'aiworkspace.blocks.noPermission')
  const fresh = store.freshness.get(wish.entity_id)
  if (isActive(store.wish.run(wish.entity_id))) return aiwsT('aiworkspace.wishcard.reason.busy')
  if (fresh?.needs_analysis) return aiwsT('aiworkspace.wishcard.reason.analysisStale')
  const reasons = fresh?.needs_coding?.reasons ?? []
  if (reasons.length) return aiwsT('aiworkspace.wishcard.reason.recodeFirst', { reason: needsCodingReasonText(reasons[0]) })
  return false
}

const wishActions: BlockAction[] = [
  { id: 'wish-run', icon: Play, primary: true, ai: true, get label() { return aiwsT('aiworkspace.wish.run') }, modes: ['edit', 'view'],
    labelFor: (context, store) => aiwsT(store.outline.all().some((e) => !e.deleted && e.derived?.wish_id === wishOf(context)) ? 'aiworkspace.wish.runAgain' : 'aiworkspace.wish.run'),
    when: (context, store) => Boolean(context.source) && holdsMacro(context.source, store.freshness.get(wishOf(context))) && !running(context, store),
    disabled: runReason,
    run: (context, store) => {
      const wish = context.source
      if (!wish) return
      runMacro(store, { wishId: wish.entity_id, title: wish.title ?? wish.name ?? wish.entity_id, cellId: context.cell.entity_id, onSurface: true, busy: false }, context.openEntity)
    } },
  { id: 'wish-cancel', icon: CircleStop, primary: true, get label() { return aiwsT('aiworkspace.wish.cancelRun') }, modes: ['edit', 'view'],
    when: running, run: (context, store) => { const run = store.wish.run(wishOf(context)); if (run) void store.wish.cancel(run) } },
]

export const wishBlock: BlockDefinition = {
  type: 'wish', version: 1, get title() { return aiwsT('aiworkspace.wish.blockTitle') }, accepts: ['buckyos.wish'], allowNoSource: false,
  defaultSize: { w: 420, h: 300 }, cost: { editor: false, html: false },
  Static: WishBlockView, Simplified: WishBlockSmall, Inspector: WishInspector, icon: Sparkles,
  // the card carries its title: a flow page draws no heading line of its own for it
  traits: { flowUntitled: true },
  get inspectorTitle() { return aiwsT('aiworkspace.wishcard.props.title') },
  sections: [{ id: 'display', get label() { return aiwsT('aiworkspace.wishcard.props.display') } }],
  configFields: [
    { key: 'show_links', kind: 'boolean', section: 'display', default: true, icon: Network, get label() { return aiwsT('aiworkspace.wishcard.props.showLinks') } },
  ],
  catalog: { group: 'ai', get description() { return aiwsT('aiworkspace.wish.blockDescription') }, needs: 'none', editAfterInsert: true, standard: true, toolPicker: 9 },
  actions: wishActions,
  // the name over the card; its run button is on the card itself
  hover: (context) => {
    const wishId = context.source?.entity_id
    if (!wishId) return []
    return [{ id: 'name', kind: 'label', at: 'top-left-out', icon: Sparkles, modes: ['edit', 'view'], freshness: wishId, label: context.payload.title ?? context.source?.title ?? aiwsT('aiworkspace.wish.blockTitle') }]
  },
  create: (args) => {
    if (args.existingSourceId) return [cellOp(args, 'wish', args.existingSourceId)]
    const title = args.title || aiwsT('aiworkspace.wish.blockTitle')
    const payload: Record<string, Json> = {
      title, prompt: args.title || '', executor: 'xllm', output_mode: 'overwrite',
      output: { container_id: args.contentFolderId, surface_id: args.surfaceId, name: aiwsT('aiworkspace.wish.resultsFolder', { title }) },
    }
    return [createOp(args.dataId, 'buckyos.wish', args.contentFolderId, args.dataOrderKey, payload, args.title || `wish-${randomId().slice(0, 6)}`), cellOp(args, 'wish', args.dataId)]
  },
}

export function registerWishBlock() {
  blockRegistry.register(wishBlock)
}

