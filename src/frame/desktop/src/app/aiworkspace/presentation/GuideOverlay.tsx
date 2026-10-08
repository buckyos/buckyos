/* The guide (第三期规划 §7.3): a website-style tour over the canvas of this window. Each step's rectangle is fitted
 * ("contain") into the canvas's clear area, everything around it is dimmed, and a bubble shows the step's caption with
 * previous / next / end. The canvas takes no input while it runs (the overlay takes every pointer and wheel; only the
 * tour's keys work). It writes no document, locks nothing and clones nothing; the progress is user state
 * `guide:<path>` (resumed next time). Frames are not cropped, no background is changed and `hide` does not apply. */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ChevronLeft, ChevronRight, X } from 'lucide-react'
import type { Json, KeyedContent, PresentationProps, ShowPathPayload, ViewportPayload } from '../api/types'
import { useLoad, useOutlineVersion, useStore, useVersion } from '../state/hooks'
import type { Camera } from '../ui/canvas/render/camera'
import { useShell } from '../ui/shell/shellContext'
import { playable, resolveSteps, type Rect } from './model'
import './presentation.css'

const PAD = 32
const BUBBLE_GAP = 14
/** About the height of the bubble with one or two lines of caption. */
const BUBBLE_H = 130

export function GuideOverlay({ camera, surfaceId }: { camera: Camera; surfaceId: string }) {
  const store = useStore()
  const shell = useShell()
  const guide = shell.guide!
  const outlineVersion = useOutlineVersion()
  const pathVersion = useVersion(`e:${guide.pathId}`)
  const load = useCallback(() => store.session.read<KeyedContent<ShowPathPayload>>(guide.pathId).then((r) => r.content.payload), [store, guide.pathId])
  const path = useLoad(load, pathVersion).data ?? null
  const steps = useMemo(() => (path ? playable(resolveSteps(store.outline, path)) : []),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal
    [path, store, outlineVersion])
  const index = Math.max(0, Math.min(guide.index, steps.length - 1))
  const step = steps[index] ?? null
  const targetVersion = useVersion(`e:${step?.targetId ?? ''}`)
  const loadCaption = useCallback(async () => {
    if (!step) return null
    const read = await store.readBatched<KeyedContent<Record<string, Json>>>(step.targetId)
    return step.kind === 'viewport' ? (read.content.payload as unknown as ViewportPayload).caption ?? null : ((read.content.payload.presentation as PresentationProps | undefined)?.caption ?? null)
  }, [store, step])
  const caption = useLoad(loadCaption, `${step?.targetId}:${targetVersion}`).data ?? null
  const [, setTick] = useState(0)
  useEffect(() => camera.onChange(() => setTick((n) => n + 1)), [camera])

  const save = useCallback((at: number, done: boolean) => {
    store.userState.set(`guide:${guide.pathId}`, { index: at, done, prompted: true })
  }, [store, guide.pathId])
  const go = useCallback((at: number) => {
    if (at < 0 || at >= steps.length) return
    save(at, false)
    shell.setGuide({ pathId: guide.pathId, index: at })
  }, [steps.length, save, shell, guide.pathId])
  const end = useCallback((done: boolean) => {
    save(index, done)
    shell.setGuide(null)
  }, [save, index, shell])

  // the step's Surface first; then fit its rectangle into the clear area
  const shown = useRef<string | null>(null)
  useEffect(() => {
    if (!step?.rect || !step.surfaceId) return
    if (step.surfaceId !== surfaceId) { shell.selectSurface(step.surfaceId); return }
    const key = `${step.step.id}:${Math.round(step.rect.x)},${Math.round(step.rect.y)},${Math.round(step.rect.w)},${Math.round(step.rect.h)}`
    if (shown.current === key) return
    const first = shown.current === null
    shown.current = key
    const target = camera.fitted(step.rect, PAD)
    if (first) camera.set(target)
    else camera.animateTo(target, 360)
  }, [step, surfaceId, camera, shell])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.isComposing) return
      // the canvas's own shortcuts do not run during a guide (§10.4)
      if (event.key === 'ArrowRight' || event.key === 'PageDown') { event.preventDefault(); event.stopImmediatePropagation(); if (index < steps.length - 1) go(index + 1); else end(true) }
      else if (event.key === 'ArrowLeft' || event.key === 'PageUp') { event.preventDefault(); event.stopImmediatePropagation(); go(index - 1) }
      else if (event.key === 'Escape') { event.preventDefault(); event.stopImmediatePropagation(); end(false) }
      else if (!(event.target instanceof Element ? event.target : null)?.closest('input, textarea, select, [contenteditable="true"]')) event.stopImmediatePropagation()
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [index, steps.length, go, end])

  // the overlay appears once the path is read: observe it from then on
  const [root, setRoot] = useState<HTMLDivElement | null>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  useEffect(() => {
    if (!root) return
    const observer = new ResizeObserver(() => setSize({ w: root.clientWidth, h: root.clientHeight }))
    observer.observe(root)
    return () => observer.disconnect()
  }, [root])

  if (!path) return null
  if (steps.length === 0) {
    return (
      <div className="aiws-guide" data-testid="aiws-guide" onPointerDown={(event) => event.stopPropagation()}>
        <div className="aiws-guide-bubble aiws-panel" style={{ left: '50%', top: '40%', transform: 'translate(-50%, -50%)' }}>
          <div>这个使用引导没有可以显示的步骤。</div>
          <div className="aiws-dialog-actions"><button type="button" onClick={() => shell.setGuide(null)}>结束</button></div>
        </div>
      </div>
    )
  }
  const onThisSurface = step?.surfaceId === surfaceId && step.rect
  const hole: Rect | null = onThisSurface ? camera.rectToScreen(step.rect!) : null
  const bubbleW = Math.min(340, Math.max(220, size.w - 24))
  // below the step, else above it, else inside its lower edge (a step that fills the canvas)
  const below = hole ? hole.y + hole.h + BUBBLE_GAP : 0
  const above = hole ? hole.y - BUBBLE_GAP - BUBBLE_H : 0
  const bubbleTop = !hole ? size.h / 2 - BUBBLE_H / 2
    : below + BUBBLE_H < size.h - 12 ? below
      : above >= 12 ? above
        : Math.max(12, Math.min(size.h - BUBBLE_H - 12, hole.y + hole.h - BUBBLE_H - 16))
  const bubbleLeft = hole ? Math.min(Math.max(12, hole.x + hole.w / 2 - bubbleW / 2), Math.max(12, size.w - bubbleW - 12)) : (size.w - bubbleW) / 2
  const last = index === steps.length - 1
  return (
    <div ref={setRoot} className="aiws-guide" data-testid="aiws-guide" data-step={step?.step.id} data-index={index} data-count={steps.length}
      onPointerDown={(event) => { if (!(event.target as HTMLElement).closest('.aiws-guide-bubble')) { event.preventDefault(); event.stopPropagation() } }}
      onWheel={(event) => { event.stopPropagation() }} onContextMenu={(event) => event.preventDefault()} onDoubleClick={(event) => event.stopPropagation()}>
      {size.w > 0 && (
        <svg className="aiws-guide-shade" data-testid="aiws-guide-shade" data-hole={hole ? `${Math.round(hole.x)},${Math.round(hole.y)},${Math.round(hole.w)},${Math.round(hole.h)}` : undefined} width={size.w} height={size.h} aria-hidden="true">
          <path fillRule="evenodd" d={`M0 0H${size.w}V${size.h}H0Z${hole ? ` M${hole.x} ${hole.y}H${hole.x + hole.w}V${hole.y + hole.h}H${hole.x}Z` : ''}`} />
        </svg>
      )}
      {hole && <div className="aiws-guide-ring" style={{ left: hole.x - 3, top: hole.y - 3, width: hole.w + 6, height: hole.h + 6 }} />}
      <div className="aiws-guide-bubble aiws-panel" role="dialog" aria-label="使用引导" data-testid="aiws-guide-bubble" style={{ left: bubbleLeft, top: bubbleTop, width: bubbleW }}>
        <div className="aiws-guide-bubble-head">
          <b className="aiws-grow">{step?.title ?? path.title ?? '使用引导'}</b>
          <span className="aiws-guide-count" data-testid="aiws-guide-count">{index + 1} / {steps.length}</span>
          <button type="button" className="aiws-icon" aria-label="结束引导" title="结束引导（Esc），停在这里" data-testid="aiws-guide-close" onClick={() => end(false)}><X size={14} /></button>
        </div>
        <div className="aiws-guide-bubble-text" data-testid="aiws-guide-caption">{caption ?? <span className="aiws-muted">（这一步没有说明）</span>}</div>
        <div className="aiws-dialog-actions">
          <button type="button" data-testid="aiws-guide-prev" disabled={index === 0} onClick={() => go(index - 1)}><ChevronLeft size={14} aria-hidden="true" /> 上一项</button>
          <span className="aiws-grow" />
          {last
            ? <button type="button" className="is-primary" data-testid="aiws-guide-done" onClick={() => end(true)}>完成</button>
            : <button type="button" className="is-primary" data-testid="aiws-guide-next" onClick={() => go(index + 1)}>下一项 <ChevronRight size={14} aria-hidden="true" /></button>}
        </div>
      </div>
    </div>
  )
}
