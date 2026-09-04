/* ── Editor shell: layout, actions, keyboard shortcuts, paste, auto-run, dialogs ── */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { parseDelimited } from '../data/csv'
import { isImageFile, readImageFile } from '../data/image'
import { cloneBlockForPaste, createFrameBlock, createImageBlock, createMetricBlock, createTableBlock, createTextBlock, createWishBlock, imageBlockHeight, looksLikeHeader, tableContentFromMatrix } from '../domain/factories'
import { newId, nowIso } from '../domain/ids'
import { activeSheet, cameraToFit, rectsIntersect, sheetBlocks, unionRect, wishStatus } from '../domain/selectors'
import type { CanvasBlock, CanvasDocument, ContextRef, Rect, TableBlockContent } from '../domain/types'
import { trackEvent } from '../events'
import { downloadText, exportDocument, exportFilename } from '../storage/export'
import type { CanvasStorageAdapter } from '../storage/indexeddb'
import type { PlacementTool } from '../store/canvas-store'
import { useCanvasEditor, useStoreState } from '../store/hooks'
import { EditorActionsContext, type EditorActions } from './actions'
import { BottomToolbar } from './BottomToolbar'
import type { CanvasController } from './canvas-controller'
import { ConfirmDialog, FeedbackDialog, ImportDialog, SettingsDialog, SnapshotsDialog, type ConfirmState } from './dialogs'
import { InfiniteCanvas } from './InfiniteCanvas'
import { LeftSidebar } from './LeftSidebar'
import { OnboardingCoach } from './OnboardingCoach'
import { PresentationPlayer } from './PresentationPlayer'
import { ToastHost } from './primitives'
import { RightPanel } from './RightPanel'
import { TopBar } from './TopBar'
import { RUNNING_STATES } from './meta'

const BLOCK_SIZE: Record<PlacementTool, { width: number; height: number }> = {
  text: { width: 320, height: 140 },
  table: { width: 560, height: 300 },
  wish: { width: 420, height: 330 },
  metric: { width: 230, height: 120 },
  frame: { width: 640, height: 420 },
  image: { width: 360, height: 268 },
}

const CLIP_MARK = 'aicanvas-blocks'

/** Nudge a candidate rect down/right until it no longer overlaps existing blocks on the active sheet. */
function findFreeRect(doc: CanvasDocument, rect: Rect): Rect {
  const others = sheetBlocks(doc, doc.activeSheetId).filter((b) => b.type !== 'frame').map((b) => b.rect)
  const overlaps = (r: Rect) => others.some((o) => rectsIntersect(r, o))
  if (!overlaps(rect)) return rect
  for (let i = 1; i <= 60; i++) {
    const down = { ...rect, y: rect.y + i * 40 }
    if (!overlaps(down)) return down
    const right = { ...rect, x: rect.x + i * 40 }
    if (!overlaps(right)) return right
  }
  return rect
}

function isEditableTarget(t: EventTarget | null): boolean {
  const el = t as HTMLElement | null
  if (!el) return false
  const tag = el.tagName
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT' || el.isContentEditable
}

export function EditorShell({ onBack, storage, pendingImportFile }: { onBack: () => void; storage: CanvasStorageAdapter; pendingImportFile?: File | null }) {
  const { store, runner } = useCanvasEditor()
  const { ui, doc } = useStoreState()
  const controllerRef = useRef<CanvasController | null>(null)
  const rootRef = useRef<HTMLDivElement>(null)
  const [importOpen, setImportOpen] = useState(Boolean(pendingImportFile))
  const [importFile, setImportFile] = useState<File | null>(pendingImportFile ?? null)
  const [feedbackOpen, setFeedbackOpen] = useState(false)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const [snapshotsOpen, setSnapshotsOpen] = useState(false)
  const [confirm, setConfirm] = useState<ConfirmState | null>(null)
  const imageInputRef = useRef<HTMLInputElement>(null)
  const imageTargetRef = useRef<string | null>(null)
  const presenting = Boolean(ui.presentation)

  const placeRect = useCallback(
    (tool: PlacementTool, point?: { x: number; y: number }, size = BLOCK_SIZE[tool]) => {
      const c = point ?? controllerRef.current?.center() ?? { x: 200, y: 200 }
      const x = point ? c.x : c.x - size.width / 2
      const y = point ? c.y : c.y - size.height / 2
      const rect = { x: Math.round(x / 8) * 8, y: Math.round(y / 8) * 8, ...size }
      return point ? rect : findFreeRect(store.doc, rect)
    },
    [store],
  )

  const addBlocks = useCallback(
    (blocks: CanvasBlock[], select = true) => {
      store.dispatch({ type: 'CREATE_BLOCKS', blocks })
      if (select) store.select(blocks.map((b) => b.id))
    },
    [store],
  )

  const actions = useMemo<EditorActions>(() => {
    const focusBlock = (id: string) => {
      const b = store.doc.blocks[id]
      if (!b) return
      if (b.sheetId !== store.doc.activeSheetId) store.dispatch({ type: 'SET_ACTIVE_SHEET', id: b.sheetId })
      store.select([id])
      requestAnimationFrame(() => controllerRef.current?.fitBlocks([id], 120))
    }
    const goToStep = (index: number) => {
      const s = store.getState().ui.presentation
      if (!s) return
      const path = store.doc.presentationPaths.find((p) => p.id === s.pathId)
      if (!path?.steps.length) return
      const i = Math.max(0, Math.min(path.steps.length - 1, index))
      store.setUi({ presentation: { pathId: s.pathId, index: i, deviated: false } })
      controllerRef.current?.animateTo(path.steps[i].camera, path.steps[i].transitionMs)
    }
    const runWish: EditorActions['runWish'] = (wishId, opts) => {
      const pre = runner.preflight(wishId)
      if (!pre.ok) {
        store.toast(pre.errors.join('；'), 'error')
        return
      }
      const go = (mode: 'replace' | 'keep') => void runner.run(wishId, { mode, adapterId: opts?.adapterId })
      if (pre.needsDecision) {
        setConfirm({
          title: '结果包含手工修改',
          body: '这个许愿格之前生成的结果已被你手工修改过。重新运行时要如何处理？\n\n· 替换：用新结果覆盖旧结果（可撤销）\n· 保留：旧结果留在原处，新结果生成在下方',
          actions: [
            { label: '保留旧结果，生成新版本', tone: 'subtle', onClick: () => go('keep') },
            { label: '替换当前结果', tone: 'primary', onClick: () => go('replace') },
          ],
        })
        return
      }
      go('replace')
    }
    const setBlockImage: EditorActions['setBlockImage'] = (blockId, file) => {
      readImageFile(file)
        .then((img) => {
          const b = store.doc.blocks[blockId]
          if (!b || b.type !== 'image') return
          const width = b.content.src ? b.rect.width : Math.min(b.rect.width, Math.max(160, img.width))
          const rect = { ...b.rect, width, height: imageBlockHeight(width, img.width, img.height) }
          const title = b.title && b.title !== '图片' ? b.title : file.name.replace(/\.[^.]+$/, '') || '图片'
          store.dispatch({
            type: 'UPDATE_BLOCK',
            id: blockId,
            patch: { title, rect, content: { ...b.content, src: img.src, naturalWidth: img.width, naturalHeight: img.height, source: { kind: 'upload', filename: file.name, bytes: img.bytes } } },
            userEdit: Boolean(b.generated),
          })
          trackEvent('file_imported', { kind: 'image', bytes: img.bytes })
        })
        .catch((e) => store.toast(e instanceof Error ? e.message : String(e), 'error'))
    }
    const insertFiles: EditorActions['insertFiles'] = (files, point) => {
      const images = files.filter(isImageFile)
      const table = files.find((f) => !isImageFile(f) && /\.(csv|tsv|txt|xlsx|xlsm)$/i.test(f.name))
      const sheetId = store.doc.activeSheetId
      const created: CanvasBlock[] = []
      images.forEach((f, i) => {
        const base = placeRect('image', point)
        const rect = { ...base, x: base.x + i * 24, y: base.y + i * 24 }
        created.push(createImageBlock({ sheetId, rect, title: f.name.replace(/\.[^.]+$/, '') || '图片' }))
      })
      if (created.length) {
        addBlocks(created)
        created.forEach((b, i) => setBlockImage(b.id, images[i]))
      }
      if (table) {
        setImportFile(table)
        setImportOpen(true)
      }
      if (!created.length && !table) store.toast('只支持图片（PNG/JPG/GIF/WebP/SVG）和表格（CSV/XLSX）文件', 'error')
    }
    return {
      openImport: (file) => {
        setImportFile(file ?? null)
        setImportOpen(true)
      },
      pickImageFor: (blockId) => {
        imageTargetRef.current = blockId
        const input = imageInputRef.current
        if (!input) return
        input.value = ''
        input.click()
      },
      setBlockImage,
      insertFiles,
      pasteClipboard: (point) => {
        const clip = store.getState().clipboard
        if (!clip.length) return
        const sheetId = store.doc.activeSheetId
        const u = unionRect(clip.map((b) => b.rect))
        const offset = point && u ? { x: point.x - u.x, y: point.y - u.y } : null
        const clones = clip.map((b) => {
          const c = cloneBlockForPaste(b, sheetId, offset ? 0 : 24)
          if (offset) c.rect = { ...c.rect, x: c.rect.x + offset.x, y: c.rect.y + offset.y }
          return c
        })
        addBlocks(clones)
      },
      openFeedback: () => setFeedbackOpen(true),
      openSettings: () => setSettingsOpen(true),
      openSnapshots: () => setSnapshotsOpen(true),
      exportJson: () => {
        downloadText(exportFilename(store.doc), exportDocument(store.doc))
        store.toast('已导出 .aicanvas.json', 'success')
      },
      runWish,
      cancelWish: (id) => runner.cancel(id),
      detachGroup: (groupId) =>
        setConfirm({
          title: '解除 AI 管理？',
          body: '解除后，这组结果会变成普通内容：不再显示"需要刷新"，许愿格重新运行时也不会更新或替换它们。内容本身保留并可自由编辑。此操作可撤销。',
          actions: [{ label: '解除 AI 管理', tone: 'primary', onClick: () => { store.dispatch({ type: 'DETACH_GENERATED', groupId }); trackEvent('result_detached') } }],
        }),
      deleteBlocks: (ids) => {
        store.dispatch({ type: 'DELETE_BLOCKS', ids })
        store.clearSelection()
      },
      duplicateBlocks: (ids) => {
        const sheetId = store.doc.activeSheetId
        const clones = ids.map((id) => store.doc.blocks[id]).filter(Boolean).map((b) => cloneBlockForPaste(b, sheetId))
        if (clones.length) addBlocks(clones)
      },
      copyBlocks: (ids) => {
        const blocks = ids.map((id) => store.doc.blocks[id]).filter(Boolean)
        store.setClipboard(blocks)
        void navigator.clipboard?.writeText(JSON.stringify({ [CLIP_MARK]: blocks })).catch(() => undefined)
        store.toast(`已复制 ${blocks.length} 个块`, 'success')
      },
      focusBlock,
      fitAll: () => controllerRef.current?.fitAll(),
      zoomBy: (f) => controllerRef.current?.zoomAtCenter(f),
      zoomTo: (z) => {
        const c = controllerRef.current
        if (!c) return
        const center = c.center()
        const v = c.viewport()
        c.animateTo({ zoom: z, x: v.width / 2 - center.x * z, y: v.height / 2 - center.y * z }, 200)
      },
      createBlockAt: (type, point) => {
        const sheetId = store.doc.activeSheetId
        const rect = placeRect(type, point)
        const block: CanvasBlock =
          type === 'text' ? createTextBlock({ sheetId, rect, title: '文本' })
          : type === 'table' ? createTableBlock({ sheetId, rect, title: '新表格' })
          : type === 'wish' ? createWishBlock({ sheetId, rect })
          : type === 'metric' ? createMetricBlock({ sheetId, rect, title: '指标', content: { label: '指标名称', value: 0 } })
          : type === 'image' ? createImageBlock({ sheetId, rect })
          : createFrameBlock({ sheetId, rect })
        addBlocks([block])
        store.setUi({ tool: null })
        if (type === 'wish') trackEvent('wish_created', { from: 'toolbar' })
        if (type === 'text') store.setUi({ editingBlockId: block.id })
        if (type === 'image') {
          imageTargetRef.current = block.id
          const input = imageInputRef.current
          if (input) {
            input.value = ''
            input.click()
          }
        }
      },
      createTableFromContent: (content, title) => {
        const size = { width: Math.min(900, 60 + content.columns.reduce((a, c) => a + (c.width ?? 120), 0)), height: Math.min(520, 80 + Math.min(content.rows.length, 15) * 28) }
        const rect = placeRect('table', undefined, size)
        const block = createTableBlock({ sheetId: store.doc.activeSheetId, rect, title, content })
        addBlocks([block])
        requestAnimationFrame(() => controllerRef.current?.fitBlocks([block.id], 100))
      },
      createWishFromTable: (tableId, range) => {
        const t = store.doc.blocks[tableId]
        if (!t || t.type !== 'table') return
        const ref: ContextRef = range ? { kind: 'tableRange', blockId: tableId, range, revision: t.dataRevision } : { kind: 'block', blockId: tableId, revision: t.dataRevision }
        const wish = createWishBlock({ sheetId: t.sheetId, rect: { x: t.rect.x + t.rect.width + 40, y: t.rect.y, ...BLOCK_SIZE.wish }, title: `许愿格：${t.title ?? '表格'}`, contextRefs: [ref] })
        addBlocks([wish])
        trackEvent('wish_created', { from: 'table' })
        requestAnimationFrame(() => controllerRef.current?.fitBlocks([tableId, wish.id], 80))
      },
      addStepFromViewport: (pathId) => {
        const path = store.doc.presentationPaths.find((p) => p.id === pathId)
        if (!path) return
        const step = { id: newId('step'), title: `步骤 ${path.steps.length + 1}`, camera: { ...activeSheet(store.doc).camera }, targetBlockIds: [], transitionMs: 600 }
        store.dispatch({ type: 'PRESENTATION_ADD_STEP', pathId, step })
        store.setUi({ selectedStepId: step.id, sidebarTab: 'presentation' })
        trackEvent('presentation_created', { from: 'viewport' })
      },
      addStepFromBlocks: (pathId, ids) => {
        const path = store.doc.presentationPaths.find((p) => p.id === pathId)
        const c = controllerRef.current
        const rects = ids.map((id) => store.doc.blocks[id]?.rect).filter(Boolean)
        const u = unionRect(rects)
        if (!path || !c || !u) return
        const first = store.doc.blocks[ids[0]]
        const step = { id: newId('step'), title: first?.title ?? `步骤 ${path.steps.length + 1}`, camera: cameraToFit(u, c.viewport(), 80), targetBlockIds: ids, transitionMs: 600 }
        store.dispatch({ type: 'PRESENTATION_ADD_STEP', pathId, step })
        store.setUi({ selectedStepId: step.id, sidebarTab: 'presentation', selectedPathId: pathId })
        store.toast(`已加入「${path.name}」第 ${path.steps.length + 1} 步`, 'success')
        trackEvent('presentation_created', { from: 'blocks' })
      },
      startPresentation: (pathId, index = 0) => {
        const path = store.doc.presentationPaths.find((p) => p.id === pathId)
        if (!path?.steps.length) return
        store.setUi({ presentation: { pathId, index, deviated: false }, selection: [], editingBlockId: null, tool: null })
        trackEvent('presentation_played', { steps: path.steps.length })
        requestAnimationFrame(() => {
          rootRef.current?.focus()
          goToStep(index)
        })
      },
      stopPresentation: () => store.setUi({ presentation: null }),
      goToStep,
      returnToStep: () => {
        const s = store.getState().ui.presentation
        if (s) goToStep(s.index)
      },
      confirm: (opts) => setConfirm(opts),
    }
  }, [store, runner, addBlocks, placeRect])

  /* ── keyboard ── */
  useEffect(() => {
    const root = rootRef.current
    if (!root) return
    // Window-level listener: focus often lands on <body> after a clicked button is replaced
    // (运行 → 取消, sidebar unmount in presentation), so we scope by activeElement instead.
    const inScope = () => {
      const a = document.activeElement
      return !a || a === document.body || root.contains(a)
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (!inScope()) return
      const state = store.getState()
      const editable = isEditableTarget(e.target)
      const mod = e.ctrlKey || e.metaKey
      if (e.key === ' ' && !editable) {
        if (!state.ui.spaceHeld) store.setUi({ spaceHeld: true })
        e.preventDefault()
        return
      }
      if (e.key === 'Escape') {
        if (state.ui.presentation) actions.stopPresentation()
        else if (state.ui.tool) store.setUi({ tool: null })
        else if (state.ui.editingBlockId) store.setUi({ editingBlockId: null })
        else if (!editable) store.clearSelection()
        return
      }
      if (state.ui.presentation) {
        if (e.key === 'ArrowRight' || e.key === 'PageDown') actions.goToStep(state.ui.presentation.index + 1)
        else if (e.key === 'ArrowLeft' || e.key === 'PageUp') actions.goToStep(state.ui.presentation.index - 1)
        else if (e.key === 'Home') actions.goToStep(0)
        else if (e.key === 'End') actions.goToStep(1e9)
        return
      }
      if (mod && (e.key === 'z' || e.key === 'Z')) {
        if (editable && !(e.target as HTMLElement).closest('.aic-cell')) return
        e.preventDefault()
        if (e.shiftKey) store.redo()
        else store.undo()
        return
      }
      if (mod && e.key === 'y') {
        if (editable) return
        e.preventDefault()
        store.redo()
        return
      }
      if (mod && e.key === 'Enter') {
        const wishId = state.ui.selection.find((id) => state.doc.blocks[id]?.type === 'wish') ?? (e.target as HTMLElement).closest('[data-block-id]')?.getAttribute('data-block-id')
        if (wishId && state.doc.blocks[wishId]?.type === 'wish') {
          e.preventDefault()
          actions.runWish(wishId)
        }
        return
      }
      if (editable) return
      if (e.key === 'Delete' || e.key === 'Backspace') {
        if (state.ui.selection.length) {
          e.preventDefault()
          actions.deleteBlocks(state.ui.selection.filter((id) => !state.doc.blocks[id]?.locked))
        }
        return
      }
      if (mod && e.key === 'c') {
        if (state.ui.selection.length) {
          e.preventDefault()
          actions.copyBlocks(state.ui.selection)
        }
        return
      }
      if (mod && e.key === 'd') {
        if (state.ui.selection.length) {
          e.preventDefault()
          actions.duplicateBlocks(state.ui.selection)
        }
        return
      }
      if (mod && e.key === 'a') {
        e.preventDefault()
        store.select(activeSheet(state.doc).blockIds)
        return
      }
      if (!mod && !e.altKey) {
        if (e.key === 'f' || e.key === 'F') return void actions.fitAll()
        if (e.key === 'p' || e.key === 'P') return void actions.createBlockAt('wish')
        if (e.key.startsWith('Arrow') && state.ui.selection.length) {
          e.preventDefault()
          const d = e.shiftKey ? 10 : 1
          const dx = e.key === 'ArrowLeft' ? -d : e.key === 'ArrowRight' ? d : 0
          const dy = e.key === 'ArrowUp' ? -d : e.key === 'ArrowDown' ? d : 0
          store.dispatch({ type: 'MOVE_BLOCKS', ids: state.ui.selection, dx, dy })
        }
      }
    }
    const onKeyUp = (e: KeyboardEvent) => {
      if (e.key === ' ') store.setUi({ spaceHeld: false })
    }
    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('keyup', onKeyUp)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('keyup', onKeyUp)
    }
  }, [store, actions])

  /* ── paste: blocks / Excel range / plain text ── */
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const root = rootRef.current
      if (!root) return
      const active = document.activeElement
      if (active && active !== document.body && !root.contains(active)) return
      if (isEditableTarget(e.target)) return
      if (store.getState().ui.presentation) return
      const imageFiles = [...(e.clipboardData?.files ?? [])].filter(isImageFile)
      if (imageFiles.length) {
        e.preventDefault()
        actions.insertFiles(imageFiles)
        return
      }
      const text = e.clipboardData?.getData('text/plain') ?? ''
      if (!text.trim()) return
      e.preventDefault()
      const sheetId = store.doc.activeSheetId
      if (text.startsWith('{') && text.includes(CLIP_MARK)) {
        try {
          const parsed = JSON.parse(text) as Record<string, CanvasBlock[]>
          const blocks = (parsed[CLIP_MARK] ?? []).map((b) => cloneBlockForPaste(b, sheetId))
          if (blocks.length) addBlocks(blocks)
          return
        } catch {
          /* fall through */
        }
      }
      const lines = text.split(/\r?\n/).filter((l) => l.trim())
      const tabular = text.includes('\t') || (lines.length >= 2 && lines.every((l) => l.includes(',')))
      if (tabular) {
        const matrix = parseDelimited(text, text.includes('\t') ? '\t' : ',')
        const content: TableBlockContent = tableContentFromMatrix(matrix, { hasHeader: looksLikeHeader(matrix), source: { kind: 'paste', importedAt: nowIso() } })
        actions.createTableFromContent(content, '粘贴的数据')
        store.toast(`已从剪贴板创建 ${content.rows.length} 行 × ${content.columns.length} 列表格`, 'success')
        trackEvent('file_imported', { kind: 'paste', rows: content.rows.length })
        return
      }
      const rect = placeRect('text')
      addBlocks([createTextBlock({ sheetId, rect, title: '文本', text })])
    }
    window.addEventListener('paste', onPaste)
    return () => window.removeEventListener('paste', onPaste)
  }, [store, actions, addBlocks, placeRect])

  /* ── auto-run for on_change wishes (page-open only) ── */
  useEffect(() => {
    const timers = new Map<string, ReturnType<typeof setTimeout>>()
    const unsub = store.subscribe(() => {
      const d = store.doc
      for (const b of Object.values(d.blocks)) {
        if (b.type !== 'wish' || b.content.refreshPolicy.mode !== 'on_change') continue
        if (RUNNING_STATES.includes(b.content.state) || timers.has(b.id)) continue
        if (wishStatus(d, b) !== 'stale') continue
        const pre = runner.preflight(b.id)
        if (!pre.ok || pre.needsDecision) continue
        timers.set(b.id, setTimeout(() => {
          timers.delete(b.id)
          const fresh = store.doc.blocks[b.id]
          if (fresh?.type === 'wish' && wishStatus(store.doc, fresh) === 'stale') void runner.run(b.id, { mode: 'replace' })
        }, 1200))
      }
    })
    return () => {
      unsub()
      timers.forEach((t) => clearTimeout(t))
    }
  }, [store, runner])

  /* ── unsaved changes guard ── */
  useEffect(() => {
    const onUnload = (e: BeforeUnloadEvent) => {
      if (store.getState().dirty || store.getState().saveStatus === 'saving') {
        e.preventDefault()
        e.returnValue = ''
      }
    }
    window.addEventListener('beforeunload', onUnload)
    return () => window.removeEventListener('beforeunload', onUnload)
  }, [store])

  const back = () => {
    void store.saveNow().finally(onBack)
  }

  return (
    <EditorActionsContext.Provider value={actions}>
      <div ref={rootRef} tabIndex={0} className="aic-root relative flex h-full w-full flex-col overflow-hidden outline-none" style={{ background: 'var(--cp-bg)' }}>
        {!presenting ? <TopBar onBack={back} /> : null}
        <div className="flex min-h-0 flex-1">
          {!presenting ? <LeftSidebar /> : null}
          <div className="relative min-w-0 flex-1">
            <InfiniteCanvas controllerRef={controllerRef} />
          </div>
          {!presenting ? <RightPanel /> : null}
        </div>
        {!presenting ? <BottomToolbar /> : null}
        <PresentationPlayer />
        {!presenting ? <OnboardingCoach /> : null}
        <ToastHost />
        <ImportDialog open={importOpen} initialFile={importFile} onClose={() => { setImportOpen(false); setImportFile(null) }} onImport={(content, title) => actions.createTableFromContent(content, title)} />
        <FeedbackDialog open={feedbackOpen} onClose={() => setFeedbackOpen(false)} canvasId={doc.id} />
        <SettingsDialog open={settingsOpen} onClose={() => setSettingsOpen(false)} />
        <SnapshotsDialog open={snapshotsOpen} onClose={() => setSnapshotsOpen(false)} storage={storage} />
        <ConfirmDialog state={confirm} onClose={() => setConfirm(null)} />
        <input
          ref={imageInputRef}
          type="file"
          accept="image/*"
          className="hidden"
          data-testid="aic-image-input"
          onChange={(e) => {
            const f = e.target.files?.[0]
            const target = imageTargetRef.current
            e.target.value = ''
            imageTargetRef.current = null
            if (f && target) actions.setBlockImage(target, f)
          }}
        />
      </div>
    </EditorActionsContext.Provider>
  )
}
