/* ── Infinite canvas: camera, pan/zoom, selection, drag/resize, marquee, connections ── */

import clsx from 'clsx'
import { useCallback, useEffect, useMemo, useRef, useState, type MutableRefObject, type PointerEvent as ReactPointerEvent } from 'react'
import { ClipboardPaste, Copy, Frame, Gauge, Image as ImageIcon, Lock, Maximize2, Sparkles, Table2, Trash2, Type, Unlink, ArrowDownToLine, ArrowUpToLine, Unlock } from 'lucide-react'
import { activeSheet, cameraToFit, rectContains, rectsIntersect, sheetBlocks, unionRect } from '../domain/selectors'
import type { Camera, Rect } from '../domain/types'
import { MIN_BLOCK_HEIGHT, MIN_BLOCK_WIDTH } from '../domain/types'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { useEditorActions } from './actions'
import { BlockView } from './BlockView'
import type { CanvasController } from './canvas-controller'
import { Menu, type MenuItem } from './primitives'
import { RUNNING_STATES } from './meta'

const MIN_ZOOM = 0.1
const MAX_ZOOM = 4
const GRID = 8

type Drag =
  | { kind: 'pan'; lastX: number; lastY: number; moved: boolean; button: number; startX: number; startY: number }
  | { kind: 'move'; ids: string[]; startX: number; startY: number; applied: { x: number; y: number }; moved: boolean; clickedId: string; shift: boolean }
  | { kind: 'resize'; id: string; dir: string; startX: number; startY: number; startRect: Rect }
  | { kind: 'marquee'; startX: number; startY: number; additive: boolean; moved: boolean }

export function InfiniteCanvas({ controllerRef }: { controllerRef: MutableRefObject<CanvasController | null> }) {
  const { store } = useCanvasEditor()
  const actions = useEditorActions()
  const { doc, ui, settings } = useStoreState()
  const sheet = activeSheet(doc)
  const cam = sheet.camera
  const rootRef = useRef<HTMLDivElement>(null)
  const dragRef = useRef<Drag | null>(null)
  const [marquee, setMarquee] = useState<Rect | null>(null)
  const [transitionMs, setTransitionMs] = useState(0)
  const [menu, setMenu] = useState<{ x: number; y: number; blockId?: string; point?: { x: number; y: number } } | null>(null)
  const [panning, setPanning] = useState(false)
  const [fileOver, setFileOver] = useState(false)
  const presenting = Boolean(ui.presentation)

  const blocks = useMemo(() => [...sheetBlocks(doc, sheet.id)].sort((a, b) => a.zIndex - b.zIndex || a.createdAt.localeCompare(b.createdAt)), [doc, sheet.id])
  const selection = useMemo(() => new Set(ui.selection), [ui.selection])

  const viewport = useCallback(() => {
    const r = rootRef.current?.getBoundingClientRect()
    return { width: r?.width ?? 800, height: r?.height ?? 600 }
  }, [])
  const camNow = useCallback(() => activeSheet(store.doc).camera, [store])
  const toCanvas = useCallback(
    (clientX: number, clientY: number) => {
      const r = rootRef.current!.getBoundingClientRect()
      const c = camNow()
      return { x: (clientX - r.left - c.x) / c.zoom, y: (clientY - r.top - c.y) / c.zoom }
    },
    [camNow],
  )

  const animateTo = useCallback(
    (camera: Camera, ms = 500) => {
      const reduced = settings.reducedMotion || window.matchMedia('(prefers-reduced-motion: reduce)').matches
      const d = reduced ? 0 : ms
      setTransitionMs(d)
      store.setCamera(camera)
      window.setTimeout(() => setTransitionMs(0), d + 30)
    },
    [store, settings.reducedMotion],
  )

  useEffect(() => {
    controllerRef.current = {
      viewport,
      camera: camNow,
      animateTo,
      center: () => {
        const v = viewport()
        const c = camNow()
        return { x: (v.width / 2 - c.x) / c.zoom, y: (v.height / 2 - c.y) / c.zoom }
      },
      toCanvas,
      fitAll: () => {
        const rects = sheetBlocks(store.doc, store.doc.activeSheetId).map((b) => b.rect)
        const u = unionRect(rects)
        animateTo(u ? cameraToFit(u, viewport(), 60) : { x: 80, y: 80, zoom: 1 }, 400)
      },
      fitBlocks: (ids, padding = 80) => {
        const u = unionRect(ids.map((id) => store.doc.blocks[id]?.rect).filter(Boolean) as Rect[])
        if (u) animateTo(cameraToFit(u, viewport(), padding), 500)
      },
      zoomAtCenter: (factor) => {
        const v = viewport()
        const c = camNow()
        const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, c.zoom * factor))
        const px = v.width / 2
        const py = v.height / 2
        animateTo({ zoom, x: px - (px - c.x) * (zoom / c.zoom), y: py - (py - c.y) * (zoom / c.zoom) }, 150)
      },
    }
    return () => {
      controllerRef.current = null
    }
  }, [controllerRef, viewport, camNow, animateTo, store, toCanvas])

  // wheel: ctrl/meta → zoom at cursor; otherwise pan
  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    const onWheel = (e: WheelEvent) => {
      e.preventDefault()
      const c = camNow()
      if (store.getState().ui.presentation) store.setUi({ presentation: { ...store.getState().ui.presentation!, deviated: true } })
      if (e.ctrlKey || e.metaKey) {
        const r = el.getBoundingClientRect()
        const px = e.clientX - r.left
        const py = e.clientY - r.top
        const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, c.zoom * Math.exp(-e.deltaY * 0.0018)))
        store.setCamera({ zoom, x: px - (px - c.x) * (zoom / c.zoom), y: py - (py - c.y) * (zoom / c.zoom) })
      } else {
        store.setCamera({ ...c, x: c.x - e.deltaX, y: c.y - e.deltaY })
      }
    }
    el.addEventListener('wheel', onWheel, { passive: false })
    return () => el.removeEventListener('wheel', onWheel)
  }, [camNow, store])

  const onPointerDown = (e: ReactPointerEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement
    // React portals (context menus) bubble synthetic events through the component tree; ignore them here.
    if (!rootRef.current?.contains(target)) return
    // clicks inside the context menu belong to the menu (it closes itself after the item's click)
    if (target.closest('[role="menu"]')) return
    if (menu) setMenu(null)
    const state = store.getState()
    const blockHit = Boolean(target.closest('[data-block-id]'))
    // right button on empty canvas: drag to pan (a plain right-click opens the insert menu on release)
    const pan = e.button === 1 || (e.button === 2 && !blockHit && !target.closest('[data-own-menu]')) || (e.button === 0 && (state.ui.spaceHeld || presenting))
    if (pan) {
      dragRef.current = { kind: 'pan', lastX: e.clientX, lastY: e.clientY, moved: false, button: e.button, startX: e.clientX, startY: e.clientY }
      if (e.button === 1) e.preventDefault() // middle button: stop autoscroll; never on left (would suppress click/dblclick)
      return
    }
    if (e.button !== 0) return
    const resizeEl = target.closest('[data-resize]') as HTMLElement | null
    const blockEl = target.closest('[data-block-id]') as HTMLElement | null
    const noDrag = target.closest('[data-no-drag]')
    const handle = target.closest('[data-drag-handle]')
    const blockId = blockEl?.dataset.blockId

    if (!blockEl && state.ui.tool) {
      const p = toCanvas(e.clientX, e.clientY)
      actions.createBlockAt(state.ui.tool, p)
      return
    }
    if (resizeEl && blockId) {
      const b = store.doc.blocks[blockId]
      if (!b || b.locked) return
      store.beginTransient()
      dragRef.current = { kind: 'resize', id: blockId, dir: resizeEl.dataset.resize!, startX: e.clientX, startY: e.clientY, startRect: { ...b.rect } }
      return
    }
    if (blockId) {
      const b = store.doc.blocks[blockId]
      if (!b) return
      if (noDrag && !handle) {
        if (!state.ui.selection.includes(blockId)) store.select([blockId])
        return
      }
      if (state.ui.editingBlockId && state.ui.editingBlockId !== blockId) store.setUi({ editingBlockId: null })
      const already = state.ui.selection.includes(blockId)
      if (e.shiftKey) {
        if (!already) store.select([blockId], { additive: true })
      } else if (!already) store.select([blockId])
      const ids = store.getState().ui.selection
      store.beginTransient()
      dragRef.current = { kind: 'move', ids, startX: e.clientX, startY: e.clientY, applied: { x: 0, y: 0 }, moved: false, clickedId: blockId, shift: e.shiftKey }
      if (!(target as HTMLElement).closest('input, textarea, select, button, [contenteditable]')) rootRef.current?.focus()
      return
    }
    // background → marquee
    dragRef.current = { kind: 'marquee', startX: e.clientX, startY: e.clientY, additive: e.shiftKey, moved: false }
    rootRef.current?.focus()
  }

  const onPointerMove = useCallback((e: PointerEvent) => {
    const d = dragRef.current
    if (!d) return
    const c = camNow()
    if (d.kind === 'pan') {
      if (!d.moved && Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < 3) return
      store.setCamera({ ...c, x: c.x + e.clientX - d.lastX, y: c.y + e.clientY - d.lastY })
      d.lastX = e.clientX
      d.lastY = e.clientY
      if (!d.moved) {
        setPanning(true)
        if (presenting) store.setUi({ presentation: { ...store.getState().ui.presentation!, deviated: true } })
      }
      d.moved = true
    } else if (d.kind === 'move') {
      const tx = (e.clientX - d.startX) / c.zoom
      const ty = (e.clientY - d.startY) / c.zoom
      if (!d.moved && Math.hypot(tx, ty) * c.zoom < 3) return
      d.moved = true
      const snap = e.altKey ? 1 : GRID
      const sx = Math.round(tx / snap) * snap
      const sy = Math.round(ty / snap) * snap
      const dx = sx - d.applied.x
      const dy = sy - d.applied.y
      if (dx || dy) {
        store.dispatch({ type: 'MOVE_BLOCKS', ids: d.ids, dx, dy })
        d.applied = { x: sx, y: sy }
      }
    } else if (d.kind === 'resize') {
      const dx = (e.clientX - d.startX) / c.zoom
      const dy = (e.clientY - d.startY) / c.zoom
      const r = { ...d.startRect }
      if (d.dir.includes('e')) r.width = Math.max(MIN_BLOCK_WIDTH, d.startRect.width + dx)
      if (d.dir.includes('s')) r.height = Math.max(MIN_BLOCK_HEIGHT, d.startRect.height + dy)
      if (d.dir.includes('w')) {
        const w = Math.max(MIN_BLOCK_WIDTH, d.startRect.width - dx)
        r.x = d.startRect.x + (d.startRect.width - w)
        r.width = w
      }
      if (d.dir.includes('n')) {
        const h = Math.max(MIN_BLOCK_HEIGHT, d.startRect.height - dy)
        r.y = d.startRect.y + (d.startRect.height - h)
        r.height = h
      }
      store.dispatch({ type: 'RESIZE_BLOCK', id: d.id, rect: r })
    } else if (d.kind === 'marquee') {
      const a = toCanvas(d.startX, d.startY)
      const b = toCanvas(e.clientX, e.clientY)
      if (!d.moved && Math.hypot(e.clientX - d.startX, e.clientY - d.startY) < 4) return
      d.moved = true
      setMarquee({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(a.x - b.x), height: Math.abs(a.y - b.y) })
    }
  }, [camNow, presenting, store, toCanvas])

  const onPointerUp = useCallback((e: PointerEvent) => {
    const d = dragRef.current
    dragRef.current = null
    if (!d) return
    if (d.kind === 'pan') {
      setPanning(false)
      if (d.button === 2 && !d.moved && !presenting) {
        // plain right-click on empty canvas → insert menu at that spot
        const r = rootRef.current?.getBoundingClientRect()
        if (r) setMenu({ x: e.clientX - r.left, y: e.clientY - r.top, point: toCanvas(e.clientX, e.clientY) })
      }
    } else if (d.kind === 'move') {
      store.endTransient()
      if (!d.moved) {
        if (d.shift) store.toggleSelect(d.clickedId)
        else store.select([d.clickedId])
      }
    } else if (d.kind === 'resize') {
      store.endTransient()
    } else if (d.kind === 'marquee') {
      if (d.moved && marquee) {
        const hits = sheetBlocks(store.doc, store.doc.activeSheetId)
          .filter((b) => (b.type === 'frame' || b.type === 'group' ? rectContains(marquee, b.rect) : rectsIntersect(marquee, b.rect)))
          .map((b) => b.id)
        store.select(hits, { additive: d.additive })
      } else if (!d.additive) {
        store.clearSelection()
        store.setUi({ tableSelection: null })
      }
      setMarquee(null)
    }
  }, [marquee, store, presenting, toCanvas])

  useEffect(() => {
    window.addEventListener('pointermove', onPointerMove)
    window.addEventListener('pointerup', onPointerUp)
    window.addEventListener('pointercancel', onPointerUp)
    return () => {
      window.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('pointerup', onPointerUp)
      window.removeEventListener('pointercancel', onPointerUp)
    }
  }, [onPointerMove, onPointerUp])

  const onContextMenu = (e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    if (!rootRef.current?.contains(target) || target.closest('[data-own-menu]')) return
    e.preventDefault()
    const blockEl = target.closest('[data-block-id]') as HTMLElement | null
    const id = blockEl?.dataset.blockId
    if (!id || presenting) return
    if (!store.getState().ui.selection.includes(id)) store.select([id])
    const r = rootRef.current!.getBoundingClientRect()
    setMenu({ x: e.clientX - r.left, y: e.clientY - r.top, blockId: id })
  }

  const canvasMenuItems = (point: { x: number; y: number }): MenuItem[] => {
    const p = { x: Math.round(point.x / GRID) * GRID, y: Math.round(point.y / GRID) * GRID }
    return [
      { label: '插入文本', icon: <Type />, onClick: () => actions.createBlockAt('text', p) },
      { label: '插入表格', icon: <Table2 />, onClick: () => actions.createBlockAt('table', p) },
      { label: '插入图片…', icon: <ImageIcon />, onClick: () => actions.createBlockAt('image', p) },
      { label: '插入许愿格', icon: <Sparkles />, onClick: () => actions.createBlockAt('wish', p) },
      { label: '插入指标', icon: <Gauge />, onClick: () => actions.createBlockAt('metric', p) },
      { label: '插入框架', icon: <Frame />, onClick: () => actions.createBlockAt('frame', p) },
      { label: '', divider: true, onClick: () => undefined },
      { label: '在此粘贴', icon: <ClipboardPaste />, disabled: store.getState().clipboard.length === 0, onClick: () => actions.pasteClipboard(p) },
      { label: '适应全部内容', icon: <Maximize2 />, onClick: () => actions.fitAll() },
    ]
  }

  const menuItems = (id: string): MenuItem[] => {
    const b = doc.blocks[id]
    if (!b) return []
    const ids = ui.selection.includes(id) ? ui.selection : [id]
    return [
      { label: '复制', icon: <Copy />, onClick: () => actions.copyBlocks(ids) },
      { label: '创建副本', icon: <Copy />, onClick: () => actions.duplicateBlocks(ids) },
      ...(b.type === 'table' ? [{ label: '基于此数据创建许愿格', icon: <Sparkles />, onClick: () => actions.createWishFromTable(b.id) }] : []),
      ...(b.type === 'group' && b.generated && !b.generated.detached ? [{ label: '解除 AI 管理', icon: <Unlink />, onClick: () => actions.detachGroup(b.id) }] : []),
      { label: '', divider: true, onClick: () => undefined },
      { label: '置顶', icon: <ArrowUpToLine />, onClick: () => store.dispatch({ type: 'REORDER_Z', id, to: 'front' }) },
      { label: '置底', icon: <ArrowDownToLine />, onClick: () => store.dispatch({ type: 'REORDER_Z', id, to: 'back' }) },
      { label: b.locked ? '解除锁定' : '锁定位置', icon: b.locked ? <Unlock /> : <Lock />, onClick: () => store.dispatch({ type: 'UPDATE_BLOCK', id, patch: { locked: !b.locked } }) },
      { label: '', divider: true, onClick: () => undefined },
      { label: ids.length > 1 ? `删除 ${ids.length} 个块` : '删除', icon: <Trash2 />, danger: true, onClick: () => actions.deleteBlocks(ids) },
    ]
  }

  // connections wish ↔ generated groups (shown when either side is selected or running)
  const connections = useMemo(() => {
    const out: Array<{ key: string; from: Rect; to: Rect; stale: boolean }> = []
    for (const b of blocks) {
      if (b.type !== 'wish') continue
      const running = RUNNING_STATES.includes(b.content.state)
      for (const gid of b.content.generatedGroupIds) {
        const g = doc.blocks[gid]
        if (!g) continue
        if (!(selection.has(b.id) || selection.has(gid) || running)) continue
        out.push({ key: `${b.id}-${gid}`, from: b.rect, to: g.rect, stale: g.generated?.status === 'stale' })
      }
    }
    return out
  }, [blocks, doc.blocks, selection])

  const cursor = panning ? 'grabbing' : ui.spaceHeld || presenting ? 'grab' : ui.tool ? 'crosshair' : 'default'

  /* files dropped from the OS: images → image blocks at the drop point, spreadsheets → import dialog */
  const hasFiles = (e: React.DragEvent) => [...e.dataTransfer.items].some((it) => it.kind === 'file')
  const onDragOver = (e: React.DragEvent) => {
    if (presenting || !hasFiles(e)) return
    e.preventDefault()
    e.dataTransfer.dropEffect = 'copy'
    if (!fileOver) setFileOver(true)
  }
  const onDrop = (e: React.DragEvent) => {
    setFileOver(false)
    if (presenting) return
    const files = [...e.dataTransfer.files]
    if (!files.length) return
    e.preventDefault()
    actions.insertFiles(files, toCanvas(e.clientX, e.clientY))
  }

  return (
    <div
      ref={rootRef}
      tabIndex={-1}
      className={clsx('aic-canvas-bg relative h-full w-full select-none overflow-hidden outline-none')}
      style={{ cursor }}
      onPointerDown={onPointerDown}
      onContextMenu={onContextMenu}
      onDragOver={onDragOver}
      onDragLeave={(e) => { if (e.currentTarget === e.target || !e.currentTarget.contains(e.relatedTarget as Node)) setFileOver(false) }}
      onDrop={onDrop}
      data-testid="aic-canvas"
    >
      <div className="aic-layer" style={{ transform: `translate(${cam.x}px, ${cam.y}px) scale(${cam.zoom})`, transition: transitionMs ? `transform ${transitionMs}ms var(--cp-ease-smooth)` : undefined, pointerEvents: presenting ? 'none' : undefined }}>
        <svg className="absolute left-0 top-0 overflow-visible" width="1" height="1" style={{ pointerEvents: 'none' }}>
          {connections.map((cn) => {
            const x1 = cn.from.x + cn.from.width / 2
            const y1 = cn.from.y + cn.from.height
            const x2 = cn.to.x + Math.min(cn.to.width / 2, 200)
            const y2 = cn.to.y
            const my = (y1 + y2) / 2
            return (
              <path key={cn.key} d={`M${x1},${y1} C${x1},${my} ${x2},${my} ${x2},${y2}`} fill="none" stroke={cn.stale ? 'var(--cp-warning)' : 'var(--aic-ai)'} strokeWidth={2 / cam.zoom} strokeDasharray={`${6 / cam.zoom} ${4 / cam.zoom}`} opacity={0.8} />
            )
          })}
        </svg>
        {blocks.map((b) => (
          <BlockView key={b.id} block={b} selected={selection.has(b.id)} editing={ui.editingBlockId === b.id} highlight={ui.highlightBlockId === b.id} presenting={presenting} />
        ))}
        {marquee ? <div className="absolute rounded border border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent)_10%,transparent)]" style={{ left: marquee.x, top: marquee.y, width: marquee.width, height: marquee.height }} /> : null}
      </div>
      {blocks.length === 0 && !presenting ? (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <div className="rounded-xl border border-dashed border-[color:var(--cp-border-opaque)] bg-[color:var(--cp-surface-opaque)] px-6 py-5 text-center text-xs text-[color:var(--cp-muted)]">
            <p className="font-semibold text-[color:var(--cp-text)]">这张 Sheet 还是空的</p>
            <p className="mt-1">从底部工具栏添加文本、表格、图片或许愿格；也可以直接粘贴 Excel 区域或图片、把图片文件拖进来，或按 <b>P</b> 创建许愿格。</p>
            <p className="mt-1">按住<b>右键</b>或<b>中键</b>拖动可平移画布，Ctrl + 滚轮缩放。</p>
          </div>
        </div>
      ) : null}
      {fileOver ? (
        <div className="pointer-events-none absolute inset-2 z-[40] flex items-center justify-center rounded-xl border-2 border-dashed border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent)_8%,transparent)]">
          <span className="rounded-md bg-[color:var(--cp-surface-opaque)] px-3 py-1.5 text-xs font-semibold text-[color:var(--cp-accent)]">松开以插入图片 / 导入表格</span>
        </div>
      ) : null}
      {menu ? <Menu at={{ x: menu.x, y: menu.y }} items={menu.blockId ? menuItems(menu.blockId) : canvasMenuItems(menu.point!)} onClose={() => setMenu(null)} /> : null}
    </div>
  )
}

