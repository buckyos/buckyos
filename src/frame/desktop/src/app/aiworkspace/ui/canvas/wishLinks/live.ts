/* Wish lines follow a running gesture (许愿格交互升级 TODO §3.9): RenderHost moves, resizes and turns Block frames in the
 * DOM without React while the pointer moves (`translate`; `left` / `top` / `width` / `height`; `rotate`) and commits once
 * at the end. This watches the style of the world layer's own frames and tells the wishes, once per animation frame, which
 * Blocks moved; a wish reads where the frames of its ends are now and repaints its lines in place. Frames this layer draws
 * carry no `data-block-id`, so its own repaints are not seen. */

import type { Rect } from '../render/camera'

export interface FrameRect { rect: Rect; rotation: number }
export type FrameRead = (id: string) => FrameRect | undefined
export type MovedListener = (moved: ReadonlySet<string>, frame: FrameRead) => void

const px = (value: string) => { const n = parseFloat(value); return Number.isFinite(n) ? n : 0 }

/** Where a frame is drawn now: its box, moved by its `translate`, and its turn (none without a box of its own). */
function readFrame(el: HTMLElement): FrameRect | undefined {
  const s = el.style
  if (!s.left || !s.top || !s.width || !s.height) return undefined
  const [tx = '0', ty = '0'] = s.translate ? s.translate.trim().split(/\s+/) : []
  return { rect: { x: px(s.left) + px(tx), y: px(s.top) + px(ty), w: px(s.width), h: px(s.height) }, rotation: s.rotate ? px(s.rotate) : 0 }
}

export class FrameWatch {
  private readonly listeners = new Set<MovedListener>()
  private readonly moved = new Set<string>()
  private observer: MutationObserver | null = null
  private world: HTMLElement | null = null
  private raf = 0

  attach(world: HTMLElement | null) {
    if (world === this.world) return
    this.observer?.disconnect()
    this.observer = null
    this.world = world
    if (!world) return
    this.observer = new MutationObserver((records) => {
      for (const r of records) {
        const el = r.target as HTMLElement
        if (el.parentElement !== world) continue
        const id = el.dataset?.blockId
        if (id) this.moved.add(id)
      }
      if (this.moved.size && !this.raf) this.raf = requestAnimationFrame(this.flush)
    })
    this.observer.observe(world, { subtree: true, attributes: true, attributeFilter: ['style'] })
  }

  dispose() {
    this.attach(null)
    if (this.raf) cancelAnimationFrame(this.raf)
    this.raf = 0
    this.moved.clear()
  }

  subscribe(listener: MovedListener): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  private readonly flush = () => {
    this.raf = 0
    const moved = new Set(this.moved)
    this.moved.clear()
    // the mounted frames by id, read once per flush and only when a listener asks
    let frames: Map<string, HTMLElement> | null = null
    const frame: FrameRead = (id) => {
      if (!frames) {
        frames = new Map()
        for (const el of this.world?.children ?? []) { const b = (el as HTMLElement).dataset?.blockId; if (b) frames.set(b, el as HTMLElement) }
      }
      const el = frames.get(id)
      return el ? readFrame(el) : undefined
    }
    for (const listener of [...this.listeners]) listener(moved, frame)
  }
}
