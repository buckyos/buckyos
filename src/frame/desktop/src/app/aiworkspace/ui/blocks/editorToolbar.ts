/* Editor tools in the near toolbar (标准对象的交互改进 §4.4, §5.4): on a free canvas the Editor of the Block
 * being edited publishes its format tools (and its lock state) into a sink the near toolbar shows; the Block
 * itself draws no toolbar, lock bar or status row. Where there is no near toolbar (flow page, data-source
 * view) there is no sink, and the same items are drawn inline by the Editor.
 *
 * Intents carry "what to open" from a toolbar button of a Block that is only selected into its Editor once it
 * mounts (e.g. the table's filter panel). */

import { createContext, useContext, useEffect, useSyncExternalStore } from 'react'
import { Emitter } from '../../state/emitter'
import type { HoverAffordance, RenderContext, ToolbarItem } from './registry'

export class ToolbarSink {
  private readonly parts = new Map<string, ToolbarItem[]>()
  private readonly emitter = new Emitter()
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.version

  set(owner: string, items: ToolbarItem[] | null) {
    if (!items || items.length === 0) { if (!this.parts.delete(owner)) return }
    else this.parts.set(owner, items)
    this.version += 1
    this.emitter.emit()
  }

  /** Every owner's items, owners separated. */
  items(): ToolbarItem[] {
    const out: ToolbarItem[] = []
    for (const [owner, items] of this.parts) {
      if (out.length > 0) out.push({ kind: 'separator', id: `sep:${owner}` })
      out.push(...items)
    }
    return out
  }
}

export const EditorToolbarContext = createContext<ToolbarSink | null>(null)

/** A free canvas (the only place with a near toolbar): Blocks there draw only their content (§4.1). */
export function useOnFreeCanvas(): boolean {
  return useContext(EditorToolbarContext) !== null
}

/** Publish `items` while mounted. Returns true when a near toolbar shows them (draw nothing inline then). */
export function useEditorToolbar(owner: string, items: ToolbarItem[] | null): boolean {
  const sink = useContext(EditorToolbarContext)
  useEffect(() => { sink?.set(owner, items) })
  useEffect(() => () => sink?.set(owner, null), [sink, owner])
  return sink !== null
}

/** An intent waiting for an Editor; `seq` tells a new request from one already handled. */
export interface Intent { value: string; seq: number }

const intents = new Map<string, Intent>()
const intentEmitter = new Emitter()
let intentSeq = 0

/** Ask the Editor of `key` (a Block id) to do `value` when it mounts, or now if it is mounted. */
export function requestIntent(key: string, value: string) {
  intentSeq += 1
  intents.set(key, { value, seq: intentSeq })
  intentEmitter.emit()
}

/** The intent waiting for `key`; the receiver handles it once (by `seq`) and consumes it. */
export function useIntent(key: string): Intent | undefined {
  return useSyncExternalStore(intentEmitter.subscribe, () => (key ? intents.get(key) : undefined))
}

export function consumeIntent(key: string, intent: Intent) {
  if (intents.get(key) === intent) intents.delete(key)
}

// ---- what a hovered or selected Block tells the canvas overlay (§4.2): its outline, resize rule and affordances

export interface BlockMeta { shape: 'rect' | 'ellipse'; aspect: 'free' | 'locked'; affordances: HoverAffordance[]; context: RenderContext }

export class BlockMetaSink {
  private readonly metas = new Map<string, BlockMeta>()
  private readonly emitter = new Emitter()
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.version
  get(id: string): BlockMeta | undefined { return this.metas.get(id) }
  set(id: string, meta: BlockMeta | null) {
    if (!meta && !this.metas.has(id)) return
    if (meta) this.metas.set(id, meta)
    else this.metas.delete(id)
    this.version += 1
    this.emitter.emit()
  }
}

export const BlockMetaContext = createContext<BlockMetaSink | null>(null)
