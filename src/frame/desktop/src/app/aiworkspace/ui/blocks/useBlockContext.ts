import { useCallback, useContext, useMemo, useSyncExternalStore } from 'react'
import type { BlockDefRead, CellPayload, EntityEnvelope, KeyedContent } from '../../api/types'
import { useEntity, useLoad, useReadOnlyReason, useStore, useVersion, useWorkspaceUi } from '../../state/hooks'
import { EditorToolbarContext } from './editorToolbar'
import { blockRegistry, modePolicy, type BlockDefinition, type CanvasMode, type DataState, type RenderContext, type Resolution, type ToolbarItem } from './registry'

const COARSE = '(pointer: coarse)'
function subscribePointer(listener: () => void) {
  const query = window.matchMedia(COARSE)
  query.addEventListener('change', listener)
  return () => query.removeEventListener('change', listener)
}
/** Touch when the primary pointer is coarse (handle sizes, affordances on selection, §4.3). */
export function usePointerType(): 'mouse' | 'touch' {
  return useSyncExternalStore(subscribePointer, () => (window.matchMedia(COARSE).matches ? 'touch' : 'mouse'), () => 'mouse')
}

export interface BlockContextOptions {
  cellId: string | null
  mode: CanvasMode
  view: RenderContext['view']
  selected?: boolean
  hovered?: boolean
  editorActive?: boolean
  onActivate?: () => void
  onDeactivate?: () => void
  size?: { w: number; h: number }
  zoom?: number
  depth?: number
}

const MISSING_DEF: BlockDefinition = {
  type: 'missing', version: 0, title: '（缺失）', accepts: [], allowNoSource: true, defaultSize: { w: 240, h: 120 }, cost: { editor: false, html: false },
  Static: () => null,
}

function dataStateOf(payload: CellPayload, source: EntityEnvelope | undefined): DataState {
  if (!payload.source_ref) return 'none'
  if (!source || source.deleted) return 'missing'
  if (!source.capabilities.includes('read')) return 'unreadable'
  if (source.degraded) return 'degraded'
  return 'ready'
}

export function useBlockContext(options: BlockContextOptions) {
  const { cellId, mode, view, depth = 0 } = options
  const store = useStore()
  const ui = useWorkspaceUi()
  const cellEntity = useEntity(cellId ?? '')
  const version = useVersion(`e:${cellId ?? ''}`)
  const load = useCallback(() => cellId ? store.readBatched<KeyedContent<CellPayload>>(cellId) : Promise.resolve(null), [store, cellId])
  const read = useLoad(load, version)
  const current = read.data?.entity_id === cellId ? read.data : null
  const cell = cellEntity ?? current
  const payload = current?.content.payload
  const source = useEntity(payload?.source_ref?.entity_id ?? '')
  const registryVersion = useSyncExternalStore(blockRegistry.subscribe, blockRegistry.snapshot)
  const defId = payload?.def_ref?.entity_id
  const defVersion = useVersion(`e:${defId ?? ''}`)
  const loadDefinition = useCallback(() => defId ? store.readBatched<BlockDefRead>(defId) : Promise.resolve(null), [store, defId])
  const definitionRead = useLoad(loadDefinition, `${defId ?? ''}:${defVersion}`)
  const documentDefinition = definitionRead.data?.entity_id === defId ? definitionRead.data?.content.payload : undefined
  const resolution = useMemo<Resolution | null>(() => {
    if (!payload?.view) return null
    const registered = blockRegistry.get(payload.view.type, payload.view.version ?? 1)
    if (registered?.definitionKind && defId && !documentDefinition && !definitionRead.error) return null
    if (registered?.definitionKind && definitionRead.data?.degraded) return { ok: false, reason: 'invalid_definition', detail: '不支持此 Block 定义的数据版本' }
    return blockRegistry.resolve(payload, source?.type_id, documentDefinition)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [payload, source?.type_id, documentDefinition, definitionRead.error, definitionRead.data?.degraded, defId, registryVersion])
  const readOnlyNow = useReadOnlyReason()
  const toolbarSink = useContext(EditorToolbarContext)
  const pointerType = usePointerType()
  if (!cell || !payload) return { context: null, resolution, error: read.error, registryVersion }
  const dataState = dataStateOf(payload, source)
  const policy = modePolicy(mode)
  // on stage only the Blocks marked "operable on stage" take input (第三期规划 §10.2)
  const editable = policy.editContent && (mode !== 'show' || payload.presentation?.live === true)
  const readOnlyReason = readOnlyNow ? readOnlyNow
    : depth > 0 ? '嵌入的 Block 只读'
      : !policy.writes ? mode === 'view' ? '查看模式：除批注外不修改文档' : '路径编辑中不修改对象内容'
        : !editable ? '放映中：这个 Block 没有开放现场操作'
        : !cell.capabilities.includes('update') ? '没有修改此 Block 的权限'
          : dataState === 'missing' || dataState === 'unreadable' || dataState === 'degraded' ? '绑定的数据不可用'
            : source && !source.capabilities.some((c) => c === 'update' || c === 'append') ? '没有修改其数据的权限' : null
  const context: RenderContext = {
    cell, payload, keyRevs: current.content.key_revs, source, definition: resolution?.ok ? resolution.definition : MISSING_DEF, documentDefinition, depth,
    mode, view, selected: Boolean(options.selected), hovered: Boolean(options.hovered), editorActive: Boolean(options.editorActive) && editable && depth === 0,
    capabilities: cell.capabilities, readOnlyReason, dataState, size: options.size ?? { w: cell.placement?.w ?? 320, h: cell.placement?.h ?? 200 }, zoom: options.zoom ?? 1,
    activateEditor: () => { if (editable && depth === 0 && resolution?.ok) options.onActivate?.() },
    deactivateEditor: () => options.onDeactivate?.(), openEntity: ui.openEntity,
    setEditorToolbar: (items: ToolbarItem[] | null) => toolbarSink?.set(`block:${cellId}`, items), pointerType,
    annotate: ui.annotate, showAnnotations: ui.showAnnotations,
  }
  const availableResolution: Resolution | null = dataState === 'missing' || dataState === 'unreadable' || dataState === 'degraded'
    ? { ok: false, reason: 'data_unavailable', detail: `绑定数据${dataState === 'missing' ? '不存在或已删除' : dataState === 'unreadable' ? '无读取权限' : '的 Schema 版本不受支持'}` }
    : resolution
  return { context, resolution: availableResolution, error: read.error, registryVersion }
}
