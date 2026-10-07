/* BlockRegistry (phase two §10.1, D6): the front end is the authority on which Renderer shows which
 * data. A Cell records `view.type` + `view.version`; the registry resolves them to a BlockDefinition
 * or says why it cannot (unknown renderer, unsupported version, data type not accepted), and the
 * BlockHost falls back to the generic read-only view. Registering a new definition touches nothing
 * else: no Shell, no gestures, no commit path, no backend.
 *
 * Mode × state are orthogonal (§10.2): the host hands every Renderer the canvas sub-mode, the view
 * (canvas / data source), selection, hover, editor activation, capabilities, the read-only reason and
 * the data availability. A definition may give edit- and view-mode specific implementations; what it
 * does not give falls back to its static renderer — never to another mode's write behaviour. */

import type { ComponentType, ReactNode } from 'react'
import type { LucideIcon } from 'lucide-react'
import { z } from 'zod'
import type { CapturedAnchor } from '../../anchors/registry'
import type { BlockDefPayload, Capability, CellPayload, EntityEnvelope, Json, Operation, Placement } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { HTML_API_VERSION } from '../extensions/htmlRuntime'

export type CanvasMode = 'edit' | 'view' | 'presentation_edit'
export const CANVAS_MODES: CanvasMode[] = ['edit', 'view', 'presentation_edit']
export const CANVAS_MODE_LABEL: Record<CanvasMode, string> = { edit: '编辑', view: '查看', presentation_edit: '播放编辑' }

export type DataState = 'ready' | 'missing' | 'unreadable' | 'degraded' | 'none'

/** What the host gives a Renderer / Editor / Inspector. Renderers never touch the client or storage. */
export interface RenderContext {
  cell: EntityEnvelope
  payload: CellPayload
  /** Version cells of the Cell's keys as last read (for `expect` on configuration writes). */
  keyRevs: Record<string, number>
  /** The bound data entity's envelope (undefined for a pure UI Block or a missing source). */
  source: EntityEnvelope | undefined
  definition: BlockDefinition
  documentDefinition?: BlockDefPayload
  depth: number
  mode: CanvasMode
  /** Where the Block is shown: on a canvas, or as the detail of the data-source view. */
  view: 'canvas' | 'source'
  selected: boolean
  hovered: boolean
  editorActive: boolean
  capabilities: Capability[]
  /** Why writes are refused right now (mode policy, capability, lock, offline…), or null. */
  readOnlyReason: string | null
  dataState: DataState
  size: { w: number; h: number }
  zoom: number
  /** Explicit activation ("edit content" / double-click in edit mode); the host mounts the Editor. */
  activateEditor: () => void
  deactivateEditor: () => void
  /** Open another entity (data-source detail or canvas focus). */
  openEntity: (entityId: string) => void
  /** An Editor hands its format tools to the near toolbar (null removes them); without a near toolbar
   * (flow page, data-source view) the call does nothing and the Editor shows its own tools. */
  setEditorToolbar: (items: ToolbarItem[] | null) => void
  pointerType: 'mouse' | 'touch'
  /** Start an annotation (null without the `comment` capability); show the annotations targeting an entity. */
  annotate: ((anchor: CapturedAnchor) => void) | null
  showAnnotations: (targetId: string) => void
}

/** One control of the near toolbar (标准对象的交互改进 §5.4). `disabled` is the reason, shown as the tooltip.
 * `panel` opens a popover with its own content (link search, lock state). */
export type ToolbarItem =
  | { kind: 'button'; id: string; /** Without an icon the label is shown (an extension's actions until it declares tools). */ icon?: LucideIcon; label: string; key?: string; active?: boolean; disabled?: string | false; ai?: boolean; run: () => void }
  | { kind: 'menu'; id: string; icon?: LucideIcon; label: string; value?: string; items: { value: string; label: string; icon?: LucideIcon }[]; onPick: (value: string) => void; disabled?: string | false }
  | { kind: 'color'; id: string; label: string; value: string | undefined; palette?: 'fill' | 'ink'; onPick: (hex: string) => void; disabled?: string | false }
  | { kind: 'panel'; id: string; icon: LucideIcon; label: string; active?: boolean; disabled?: string | false; render: (close: () => void) => ReactNode }
  | { kind: 'separator'; id: string }

/** What hovering a Block floats next to it (§4.2): a name label or an icon button, placed by the host. */
export interface HoverAffordance {
  id: string
  kind: 'label' | 'button'
  /** Outside above the left edge / inside the top-right corner / outside below the middle / inside the bottom-right corner. */
  at: 'top-left-out' | 'top-right-in' | 'bottom-out' | 'bottom-right-in'
  label: string
  icon?: LucideIcon
  /** Show this entity's freshness badge after the label. */
  freshness?: string
  /** A BlockAction id, or `run`. */
  action?: string
  run?: (context: RenderContext) => void
  /** Sub-modes where it shows; edit only by default. */
  modes?: CanvasMode[]
}

export interface BlockAction {
  id: string
  label: string
  /** Sub-modes where the action is offered; the policy of the mode still applies (§10.2). */
  modes: CanvasMode[]
  /** Both canvas and source views unless narrowed. */
  views?: ('canvas' | 'source')[]
  /** Available with these capabilities on the Block (and its source when `onSource`). */
  needs?: Capability[]
  onSource?: boolean
  when?: (context: RenderContext) => boolean
  run: (context: RenderContext, store: WorkspaceStore) => void | Promise<void>
  /** Shown as a keyboard hint; the host binds it while the Block is selected. */
  key?: string
}

/** Insert catalog groups (UI improvement §6.3): standard objects, samples kept apart, workspace extensions. */
export type CatalogGroup = 'text' | 'data' | 'layout' | 'ai' | 'sample' | 'extension'
export const CATALOG_GROUP_LABEL: Record<CatalogGroup, string> = { text: '文本与便签', data: '数据与媒体', layout: '布局', ai: 'AI', sample: '样本', extension: '工作区扩展' }

export interface CatalogInfo {
  group: CatalogGroup
  description: string
  /** What insertion needs first: nothing (the definition creates its own data, or is pure UI), existing data of
   * an accepted type, a file, or a workspace Block definition (`buckyos.block-def`). */
  needs: 'none' | 'data' | 'file' | 'definition'
  /** Inserted without a title prompt, then the editor opens at once (text, notes, wishes). */
  editAfterInsert?: boolean
  /** Offered directly on the object toolbar. */
  standard?: boolean
}

export interface BlockDefinition {
  /** Stable renderer id (`view.type`), e.g. `table`, `frame`, `acme.kpi`. */
  type: string
  version: number
  title: string
  /** Data types this Renderer accepts; `[]` with `allowNoSource` for pure UI Blocks. */
  accepts: string[]
  allowNoSource: boolean
  defaultSize: { w: number; h: number }
  /** Declared cost: the host limits how many editors / HTML runtimes are mounted at once (§9.3 rule 8). */
  cost: { editor: boolean; html: boolean }
  /** Static content (required): no media playback, no code, no wish execution; updates with the data. */
  Static: ComponentType<RenderContext>
  /** Low-detail rendering when the Block is small on screen (§9.3 rule 7); the host draws a placeholder otherwise. */
  Simplified?: ComponentType<RenderContext>
  /** View-mode specific reading interaction (links, previews, reading focus). */
  View?: ComponentType<RenderContext>
  /** Mounted only after explicit activation in edit mode, or directly in the data-source view. */
  Editor?: ComponentType<RenderContext>
  /** Property editing for the side panel; the host places it. */
  Inspector?: ComponentType<RenderContext>
  actions?: BlockAction[]
  /** Pure UI Blocks live in the BlockTree only (frame, shape); content Blocks pair with a data entity (§4.2). */
  pureUi?: boolean
  /** How "insert" creates this Block (data entity + Cell in one commit, or the Cell alone). */
  create?: (args: CreateArgs) => Operation[]
  /** Config schema for the generic inspector (simple key → control). */
  configFields?: ConfigField[]
  configSchema?: Json
  definitionKind?: BlockDefPayload['kind']
  /** Listed by the insert catalog (and the toolbars built on it); a definition without it is not offered for insertion. */
  catalog?: CatalogInfo
  /** The host frame draws nothing (§4.1); `clip` (default) cuts off and scrolls content larger than the Block,
   * `none` lets it draw outside (titles, shadows). */
  chrome?: 'none' | 'clip'
  /** The outline the hover state follows (and, later, connector points). */
  shape?: 'rect' | 'ellipse' | ((context: RenderContext) => 'rect' | 'ellipse')
  /** `locked`: corner handles keep the proportions (Shift frees them); `free`: Shift keeps them. */
  resize?: { aspect?: 'free' | 'locked' }
  /** Hover affordances (§4.2); without it a Block with data shows the data's name. */
  hover?: (context: RenderContext, store: WorkspaceStore) => HoverAffordance[]
  /** The type section of the near toolbar for the current state (§5.2). */
  toolbar?: (context: RenderContext, store: WorkspaceStore) => ToolbarItem[]
}

export interface ConfigField { key: string; label: string; kind: 'text' | 'number' | 'select' | 'boolean' | 'color'; options?: { value: string; label: string }[] }

export interface CreateArgs {
  store: WorkspaceStore
  surfaceId: string
  contentFolderId: string
  cellId: string
  dataId: string
  parentId: string
  orderKey: string
  dataOrderKey: string
  placement: Placement
  title?: string
  /** For "add existing data to the canvas": no data entity is created. */
  existingSourceId?: string
  config?: Record<string, Json>
}

export type Resolution =
  | { ok: true; definition: BlockDefinition; warning?: string }
  | { ok: false; reason: 'unknown_renderer' | 'unsupported_version' | 'type_not_accepted' | 'source_required' | 'definition_missing' | 'invalid_definition' | 'unsupported_api' | 'invalid_config' | 'data_unavailable'; detail: string; definition?: BlockDefinition }

class BlockRegistry {
  private readonly defs = new Map<string, Map<number, BlockDefinition>>()
  private readonly listeners = new Set<() => void>()
  private version = 0

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }
  snapshot = () => this.version

  private changed() {
    this.version += 1
    for (const listener of [...this.listeners]) listener()
  }

  register(definition: BlockDefinition): () => void {
    let versions = this.defs.get(definition.type)
    if (!versions) { versions = new Map(); this.defs.set(definition.type, versions) }
    versions.set(definition.version, definition)
    this.changed()
    return () => this.unregister(definition.type, definition.version)
  }

  unregister(type: string, version: number) {
    const versions = this.defs.get(type)
    if (!versions?.delete(version)) return
    if (versions.size === 0) this.defs.delete(type)
    this.changed()
  }

  get(type: string, version?: number): BlockDefinition | undefined {
    const versions = this.defs.get(type)
    if (!versions) return undefined
    if (version !== undefined) return versions.get(version)
    let best: BlockDefinition | undefined
    for (const def of versions.values()) if (!best || def.version > best.version) best = def
    return best
  }

  list(): BlockDefinition[] {
    const out: BlockDefinition[] = []
    for (const versions of this.defs.values()) for (const def of versions.values()) out.push(def)
    return out.sort((a, b) => a.type.localeCompare(b.type) || a.version - b.version)
  }

  /** Definitions that can show a data entity of `typeId` (for "add to canvas" / "add a view"). */
  forSource(typeId: string): BlockDefinition[] {
    return this.list().filter((def) => def.accepts.includes(typeId))
  }

  /** Resolve what a Cell asks for. A version the registry lacks is never silently replaced (§10.3). */
  resolve(payload: CellPayload, sourceType: string | undefined, documentDefinition?: BlockDefPayload): Resolution {
    const versions = this.defs.get(payload.view.type)
    if (!versions) return { ok: false, reason: 'unknown_renderer', detail: `未知的渲染器 ${payload.view.type}` }
    const wanted = payload.view.version ?? 1
    let definition = versions.get(wanted)
    if (!definition) {
      const latest = this.get(payload.view.type)
      return { ok: false, reason: 'unsupported_version', detail: `渲染器 ${payload.view.type} 的版本 ${wanted} 在此版本中不可用${latest ? `（可用：${[...versions.keys()].join('、')}）` : ''}`, definition: latest }
    }
    if (definition.definitionKind) {
      if (!documentDefinition) return { ok: false, reason: 'definition_missing', detail: 'Block 定义不存在或无法读取' }
      if (documentDefinition.kind !== definition.definitionKind) return { ok: false, reason: 'invalid_definition', detail: `需要 ${definition.definitionKind} 定义` }
      if (documentDefinition.kind === 'html' && (documentDefinition.html?.api_version ?? HTML_API_VERSION) !== HTML_API_VERSION) {
        return { ok: false, reason: 'unsupported_api', detail: `不支持 HTML API 版本 ${documentDefinition.html?.api_version}` }
      }
      definition = {
        ...definition,
        title: documentDefinition.title ?? definition.title,
        accepts: documentDefinition.accepts ?? definition.accepts,
        allowNoSource: documentDefinition.allow_no_source ?? definition.allowNoSource,
        defaultSize: documentDefinition.default_size ?? definition.defaultSize,
        configSchema: documentDefinition.config_schema,
      }
    }
    if (definition.configSchema !== undefined) {
      try {
        const schema = definition.configSchema
        if (schema === null || Array.isArray(schema) || (typeof schema !== 'object' && typeof schema !== 'boolean')) throw new Error('config_schema 必须是 JSON Schema 对象或布尔值')
        const { snapshot: _snapshot, ...config } = payload.config ?? {}
        void _snapshot
        const parsed = z.fromJSONSchema(schema).safeParse(config)
        if (!parsed.success) return { ok: false, reason: 'invalid_config', detail: `Block 配置不符合定义：${parsed.error.issues.map((i) => `${i.path.join('.')}: ${i.message}`).join('；')}` }
      } catch (error) {
        return { ok: false, reason: 'invalid_definition', detail: `无法解析配置 Schema：${error instanceof Error ? error.message : String(error)}` }
      }
    }
    if (!payload.source_ref) {
      if (!definition.allowNoSource) return { ok: false, reason: 'source_required', detail: `${definition.title} 需要绑定数据`, definition }
      return { ok: true, definition }
    }
    if (sourceType && !definition.accepts.includes(sourceType)) {
      return { ok: false, reason: 'type_not_accepted', detail: `${definition.title} 不支持数据类型 ${sourceType}`, definition }
    }
    return { ok: true, definition }
  }
}

export const blockRegistry = new BlockRegistry()

/** Mode policy (§10.2): what each canvas sub-mode allows, regardless of which buttons are shown. */
export interface ModePolicy {
  select: boolean
  layout: boolean
  editContent: boolean
  insert: boolean
  annotate: boolean
  readingInteraction: boolean
  /** Document writes allowed at all (besides annotations in view mode). */
  writes: boolean
}

export function modePolicy(mode: CanvasMode): ModePolicy {
  switch (mode) {
    case 'edit':
      return { select: true, layout: true, editContent: true, insert: true, annotate: true, readingInteraction: false, writes: true }
    case 'view':
      return { select: true, layout: false, editContent: false, insert: false, annotate: true, readingInteraction: true, writes: false }
    case 'presentation_edit':
      return { select: false, layout: false, editContent: false, insert: false, annotate: false, readingInteraction: false, writes: false }
  }
}
