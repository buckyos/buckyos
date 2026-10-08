/* The presentation model (第三期规划 §5.3, §6, §9): pure functions the path editor, the stage, the guide and the
 * prompter share. Every step resolves to one world rectangle R on one Surface; the stage is the rectangle S the
 * path's stage size takes when fitted ("contain") into a window; the camera is the one that makes R fill S. Window
 * sizes change S only, never R, so every window shows the same content with more or less letterboxing. */

import type { EntityEnvelope, ShowPathPayload, ShowStep, StageSize, StepTransition } from '../api/types'
import type { OutlineModel } from '../state/outline'

export interface Rect { x: number; y: number; w: number; h: number }
export interface View { x: number; y: number; zoom: number }

export const DEFAULT_STAGE: StageSize = { w: 1920, h: 1080 }
export const STAGE_PRESETS: { id: string; label: string; stage: StageSize }[] = [
  { id: '16:9', label: '16:9（1920 × 1080）', stage: { w: 1920, h: 1080 } },
  { id: '4:3', label: '4:3（1440 × 1080）', stage: { w: 1440, h: 1080 } },
]
export const DEFAULT_BACKGROUND = '#ffffff'
export const MIN_ZOOM = 0.05
export const MAX_ZOOM = 4

/** Stage aspects closer than this count as equal (rounding of placements). */
const RATIO_SLACK = 0.01

export function sameRatio(a: { w: number; h: number }, b: { w: number; h: number }): boolean {
  return Math.abs(a.w / a.h - b.w / b.h) <= RATIO_SLACK * (b.w / b.h)
}

export function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom))
}

/** The world rectangle a Viewport shows on a stage of `stage` (§5.1). */
export function viewportRect(center: { x: number; y: number }, zoom: number, stage: StageSize): Rect {
  const w = stage.w / zoom
  const h = stage.h / zoom
  return { x: center.x - w / 2, y: center.y - h / 2, w, h }
}

/** A Viewport for the world rectangle `r` on a stage of `stage` (the inverse; `r` has the stage's aspect). */
export function viewportOf(r: Rect, stage: StageSize): { center: { x: number; y: number }; zoom: number } {
  return { center: { x: Math.round(r.x + r.w / 2), y: Math.round(r.y + r.h / 2) }, zoom: Number(clampZoom(stage.w / r.w).toFixed(4)) }
}

/** World rectangle and rotation of a Block, its placement made absolute through enclosing groups; null off a Surface. */
export function worldRectOf(outline: OutlineModel, id: string): { surfaceId: string; rect: Rect; rotation: number } | null {
  const entity = outline.get(id)
  if (!entity?.placement) return null
  const rect = { x: entity.placement.x, y: entity.placement.y, w: entity.placement.w, h: entity.placement.h }
  let parent = entity.parent_id ? outline.get(entity.parent_id) : undefined
  for (let i = 0; parent && i < 64; i++) {
    if (parent.kind === 'surface') return { surfaceId: parent.entity_id, rect, rotation: entity.placement.rotation ?? 0 }
    if (parent.kind !== 'group') return null
    rect.x += parent.placement?.x ?? 0
    rect.y += parent.placement?.y ?? 0
    parent = parent.parent_id ? outline.get(parent.parent_id) : undefined
  }
  return null
}

/** The Frame keeping its centre and width at the stage's aspect (§4.1: one commit when a Frame joins a path). */
export function frameAtRatio(rect: Rect, stage: StageSize): Rect {
  const h = Math.max(1, Math.round((rect.w * stage.h) / stage.w))
  return { x: rect.x, y: Math.round(rect.y + rect.h / 2 - h / 2), w: rect.w, h }
}

export type StepProblem = 'missing' | 'surface' | 'flow' | 'rotated'
export const STEP_PROBLEM_TEXT: Record<StepProblem, string> = {
  missing: '目标已删除或无权查看',
  surface: '目标所在的画布已不存在',
  flow: '目标不在自由画布上',
  rotated: 'Frame 已旋转：放映时按未旋转显示',
}

/** One step as a show needs it: where it is, what it is called, and whether it can be shown. */
export interface ResolvedStep {
  step: ShowStep
  /** Position in the path (all steps). */
  index: number
  kind: 'frame' | 'viewport'
  targetId: string
  target: EntityEnvelope | undefined
  surfaceId: string | null
  rect: Rect | null
  title: string
  /** A Frame whose aspect differs from the stage (shown fitted, letterboxed). */
  ratioMismatch: boolean
  problem: StepProblem | null
}

export function stepTitle(step: ShowStep, target: EntityEnvelope | undefined, n: number): string {
  return step.title || target?.title || target?.name || `第 ${n} 步`
}

export function resolveStep(outline: OutlineModel, path: ShowPathPayload, step: ShowStep, index: number): ResolvedStep {
  const target = outline.get(step.target.entity_id)
  const base = { step, index, kind: step.target.kind, targetId: step.target.entity_id, target, title: stepTitle(step, target, index + 1), ratioMismatch: false }
  if (!target || target.deleted) return { ...base, surfaceId: null, rect: null, problem: 'missing' }
  const surfaceFree = (id: string | null | undefined) => {
    const s = id ? outline.get(id) : undefined
    return !s || s.deleted ? 'surface' : s.layout?.mode !== 'free' ? 'flow' : null
  }
  if (step.target.kind === 'frame') {
    const placed = worldRectOf(outline, target.entity_id)
    if (!placed) return { ...base, surfaceId: null, rect: null, problem: 'surface' }
    const problem = surfaceFree(placed.surfaceId) ?? (placed.rotation ? 'rotated' : null)
    return { ...base, surfaceId: placed.surfaceId, rect: placed.rect, ratioMismatch: !sameRatio(placed.rect, path.stage), problem }
  }
  const vp = target.viewport
  if (!vp?.center || !vp.zoom) return { ...base, surfaceId: null, rect: null, problem: 'missing' }
  return { ...base, surfaceId: vp.surface_id, rect: viewportRect(vp.center, vp.zoom, path.stage), problem: surfaceFree(vp.surface_id) }
}

export function resolveSteps(outline: OutlineModel, path: ShowPathPayload): ResolvedStep[] {
  return path.steps.map((step, index) => resolveStep(outline, path, step, index))
}

/** What a show plays: enabled steps whose target resolves (a rotated Frame plays, upright). Dangling ones are skipped. */
export function playable(steps: ResolvedStep[]): ResolvedStep[] {
  return steps.filter((s) => s.step.enabled !== false && s.rect !== null && s.surfaceId !== null && (s.problem === null || s.problem === 'rotated'))
}

/** S: the stage fitted ("contain") and centred into `area` (screen px). */
export function fitStage(area: Rect, stage: StageSize): Rect {
  const zoom = Math.min(area.w / stage.w, area.h / stage.h)
  const w = stage.w * zoom
  const h = stage.h * zoom
  return { x: area.x + (area.w - w) / 2, y: area.y + (area.h - h) / 2, w, h }
}

/** The camera showing world rectangle `r` fitted and centred in screen rectangle `s`, and where `r` lands (the hole of the
 * mask: a Frame of another aspect is letterboxed inside the stage). */
export function stageView(r: Rect, s: Rect): { view: View; hole: Rect } {
  const zoom = clampZoom(Math.min(s.w / r.w, s.h / r.h))
  const w = r.w * zoom
  const h = r.h * zoom
  const hole = { x: s.x + (s.w - w) / 2, y: s.y + (s.h - h) / 2, w, h }
  return { view: { x: r.x - hole.x / zoom, y: r.y - hole.y / zoom, zoom }, hole }
}

/** The world rectangle the screen rectangle `s` shows under `view`. */
export function worldOf(s: Rect, view: View): Rect {
  return { x: view.x + s.x / view.zoom, y: view.y + s.y / view.zoom, w: s.w / view.zoom, h: s.h / view.zoom }
}

export function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

/** The transition actually played into `to` (§9.2): the step's own choice or the default, `fly` only within one Surface
 * and never with reduced motion. */
export function transitionFor(from: ResolvedStep | null, to: ResolvedStep, reducedMotion: boolean): Exclude<StepTransition, 'auto'> {
  const asked = to.step.transition ?? 'auto'
  if (asked === 'cut') return 'cut'
  if (!from) return 'fade'
  const sameSurface = from.surfaceId === to.surfaceId
  let chosen: Exclude<StepTransition, 'auto'> = asked === 'auto' ? (sameSurface && (from.kind === 'viewport' || to.kind === 'viewport') ? 'fly' : 'fade') : asked
  if (chosen === 'fly' && (!sameSurface || reducedMotion)) chosen = 'fade'
  return chosen
}

export const FADE_MS = 150
const FLY_MIN_MS = 400
const FLY_MAX_MS = 1200
const FLY_MS_PER_UNIT = 380
const RHO = 1.4

/** A camera "centre + visible width" in world units. */
export interface Eye { cx: number; cy: number; w: number }

export function easeInOut(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2
}

/** Smooth zoom between two eyes (van Wijk & Nuij, "Smooth and efficient zooming and panning", ρ = 1.4; the same
 * construction as d3-interpolate's `interpolateZoom`): far apart it zooms out, pans and zooms in; close by it is a
 * plain pan and zoom. The duration follows the path length, within 0.4–1.2 s. */
export function flyPath(from: Eye, to: Eye): { duration: number; at: (t: number) => Eye } {
  const rho2 = RHO * RHO
  const rho4 = rho2 * rho2
  const dx = to.cx - from.cx
  const dy = to.cy - from.cy
  const d2 = dx * dx + dy * dy
  let length: number
  let at: (s: number) => Eye
  if (d2 < 1e-12) {
    length = Math.log(to.w / from.w) / RHO
    at = (s) => ({ cx: from.cx + s * dx, cy: from.cy + s * dy, w: from.w * Math.exp(RHO * s * length) })
  } else {
    const d1 = Math.sqrt(d2)
    const b0 = (to.w * to.w - from.w * from.w + rho4 * d2) / (2 * from.w * rho2 * d1)
    const b1 = (to.w * to.w - from.w * from.w - rho4 * d2) / (2 * to.w * rho2 * d1)
    const r0 = Math.log(Math.sqrt(b0 * b0 + 1) - b0)
    const r1 = Math.log(Math.sqrt(b1 * b1 + 1) - b1)
    length = (r1 - r0) / RHO
    const coshR0 = Math.cosh(r0)
    const sinhR0 = Math.sinh(r0)
    at = (s) => {
      const ss = s * length
      const u = (from.w / (rho2 * d1)) * (coshR0 * Math.tanh(RHO * ss + r0) - sinhR0)
      return { cx: from.cx + u * dx, cy: from.cy + u * dy, w: (from.w * coshR0) / Math.cosh(RHO * ss + r0) }
    }
  }
  const duration = Math.min(FLY_MAX_MS, Math.max(FLY_MIN_MS, Math.abs(length) * FLY_MS_PER_UNIT))
  return { duration, at: (t) => (t >= 1 ? to : t <= 0 ? from : at(easeInOut(t))) }
}

/** The eye of a camera looking through screen rectangle `s`, and the camera of an eye through `s`. */
export function eyeOf(view: View, s: Rect): Eye {
  return { cx: view.x + (s.x + s.w / 2) / view.zoom, cy: view.y + (s.y + s.h / 2) / view.zoom, w: s.w / view.zoom }
}
export function viewOfEye(eye: Eye, s: Rect): View {
  const zoom = s.w / eye.w
  return { x: eye.cx - (s.x + s.w / 2) / zoom, y: eye.cy - (s.y + s.h / 2) / zoom, zoom }
}

// ---- step commands (§6.3): every edit of a path is a list of commands by step id, replayed on the newest steps

export type StepCommand =
  /** `after`: a step id, `null` for the first place, `'$end'` for the last. */
  | { kind: 'insert'; after: string | null; steps: ShowStep[] }
  | { kind: 'move'; id: string; after: string | null }
  | { kind: 'remove'; id: string }
  /** `undefined` values remove the key. */
  | { kind: 'update'; id: string; patch: Partial<Omit<ShowStep, 'id'>> }

export const END = '$end'

/** Apply `commands` to `steps`; a command whose step (or anchor) is gone cannot be replayed: its reason is returned. */
export function applyStepCommands(steps: ShowStep[], commands: StepCommand[]): ShowStep[] | { unreplayable: string } {
  let out = steps.map((s) => ({ ...s }))
  const indexOf = (id: string) => out.findIndex((s) => s.id === id)
  const place = (after: string | null): number | null => {
    if (after === null) return 0
    if (after === END) return out.length
    const i = indexOf(after)
    return i < 0 ? null : i + 1
  }
  for (const command of commands) {
    switch (command.kind) {
      case 'insert': {
        const at = place(command.after)
        if (at === null) return { unreplayable: '插入位置所在的步骤已被删除' }
        const fresh = command.steps.filter((s) => indexOf(s.id) < 0)
        out = [...out.slice(0, at), ...fresh, ...out.slice(at)]
        break
      }
      case 'move': {
        const i = indexOf(command.id)
        if (i < 0) return { unreplayable: '要移动的步骤已被删除' }
        const [moved] = out.splice(i, 1)
        if (command.after === command.id) return { unreplayable: '无效的移动' }
        const at = place(command.after)
        if (at === null) { out.splice(i, 0, moved); return { unreplayable: '移动的目标位置已被删除' } }
        out.splice(at, 0, moved)
        break
      }
      case 'remove': {
        const i = indexOf(command.id)
        if (i >= 0) out.splice(i, 1)
        break
      }
      case 'update': {
        const i = indexOf(command.id)
        if (i < 0) return { unreplayable: '要修改的步骤已被删除' }
        const next: Record<string, unknown> = { ...out[i] }
        for (const [key, value] of Object.entries(command.patch)) {
          if (value === undefined) delete next[key]
          else next[key] = value
        }
        out[i] = next as unknown as ShowStep
        break
      }
    }
  }
  return out
}

/** Paths of the workspace in tree order, presentation paths first or guides only. */
export function pathsOf(outline: OutlineModel, purpose?: 'presentation' | 'guide'): EntityEnvelope[] {
  return outline.childrenOf('shows').filter((e) => e.type_id === 'buckyos.show-path' && (!purpose || e.show_path?.purpose === purpose))
}

/** Surfaces a path's steps show (for "operable Blocks" and pre-mounting). */
export function surfacesOfSteps(steps: ResolvedStep[]): string[] {
  return [...new Set(steps.flatMap((s) => (s.surfaceId ? [s.surfaceId] : [])))]
}

/** Does any Block on these Surfaces take operations on stage (`presentation.live`)? */
export function hasLiveBlocks(outline: OutlineModel, surfaceIds: string[]): boolean {
  return surfaceIds.some((id) => outline.descendants(id).some((e) => e.live === true && !e.deleted))
}
