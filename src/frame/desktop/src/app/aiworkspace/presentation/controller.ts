/* The show controller (第三期规划 §12): every control entry — keys, the control bar, a prompter, touch — turns into a
 * command `{ command_id, kind, args }` handled here, on the stage, which then publishes the new state. A transition
 * records only the target step and the transition actually used (§9: no intermediate frames). */

import type { ShowCommand, ShowCommandKind, ShowState } from '../api/types'
import { randomId } from '../api/ids'
import { transitionFor, type ResolvedStep } from './model'

export type Transition = 'fly' | 'fade' | 'cut'

export interface StageState {
  index: number
  black: boolean
  /** The camera was moved by hand (§9.5): the step stays, "回到本步骤" flies back. */
  free: boolean
  pointer: { x: number; y: number } | null
  /** How the current step was reached, and a counter of navigations (each one animates once). */
  transition: Transition
  nonce: number
}

const SEEN_COMMANDS = 200

export function command(kind: ShowCommandKind, args?: ShowCommand['args']): ShowCommand {
  return { command_id: randomId('cmd'), kind, ...(args ? { args } : {}) }
}

export class ShowController {
  private steps: ResolvedStep[]
  private state: StageState
  private readonly listeners = new Set<() => void>()
  private readonly seen: string[] = []
  private readonly reducedMotion: () => boolean

  constructor(steps: ResolvedStep[], startIndex: number, reducedMotion: () => boolean) {
    this.steps = steps
    this.reducedMotion = reducedMotion
    this.state = { index: Math.max(0, Math.min(startIndex, steps.length - 1)), black: false, free: false, pointer: null, transition: 'fade', nonce: 1 }
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }
  snapshot = (): StageState => this.state

  playable(): ResolvedStep[] { return this.steps }
  current(): ResolvedStep | null { return this.steps[this.state.index] ?? null }
  next(): ResolvedStep | null { return this.steps[this.state.index + 1] ?? null }

  private set(patch: Partial<StageState>) {
    this.state = { ...this.state, ...patch }
    for (const listener of [...this.listeners]) listener()
  }

  private goTo(index: number) {
    if (this.steps.length === 0) return
    const target = Math.max(0, Math.min(index, this.steps.length - 1))
    const from = this.current()
    const to = this.steps[target]
    // the same step again (back from free browsing) flies back on the same Surface
    const transition: Transition = target === this.state.index && from ? (this.reducedMotion() ? 'fade' : 'fly') : transitionFor(from, to, this.reducedMotion())
    if (target === this.state.index && !this.state.free) return
    this.set({ index: target, free: false, transition, nonce: this.state.nonce + 1 })
  }

  /** Handle one command; a command id seen before is ignored (a resend over the relay). */
  apply(cmd: ShowCommand) {
    if (this.seen.includes(cmd.command_id)) return
    this.seen.push(cmd.command_id)
    if (this.seen.length > SEEN_COMMANDS) this.seen.shift()
    switch (cmd.kind) {
      case 'next': this.goTo(this.state.index + 1); break
      case 'prev': this.goTo(this.state.index - 1); break
      case 'first': this.goTo(0); break
      case 'last': this.goTo(this.steps.length - 1); break
      case 'goto': {
        const id = cmd.args?.step_id
        const i = this.steps.findIndex((s) => s.step.id === id)
        if (i >= 0) this.goTo(i)
        break
      }
      case 'black': this.set({ black: typeof cmd.args?.on === 'boolean' ? cmd.args.on : !this.state.black }); break
      case 'back': if (this.state.free) this.goTo(this.state.index); break
    }
  }

  /** The camera was moved by hand. */
  setFree() {
    if (!this.state.free) this.set({ free: true })
  }

  setPointer(pointer: { x: number; y: number } | null) {
    if (pointer === null && this.state.pointer === null) return
    this.set({ pointer })
  }

  /** The path or its targets changed (a read-only show follows a live document): keep the current step when it is still
   * there, otherwise stay at the same position. */
  updateSteps(steps: ResolvedStep[]) {
    if (steps === this.steps) return
    const id = this.current()?.step.id
    this.steps = steps
    const kept = steps.findIndex((s) => s.step.id === id)
    const index = kept >= 0 ? kept : Math.min(this.state.index, Math.max(0, steps.length - 1))
    if (index !== this.state.index || kept < 0) this.set({ index, transition: 'cut', nonce: this.state.nonce + 1 })
    else this.set({})
  }

  /** What the relay publishes. */
  published(showId: string, pathId: string, startedAt: string): ShowState {
    const s = this.state
    return { show_id: showId, path_id: pathId, step_id: this.current()?.step.id ?? null, index: s.index, count: this.steps.length, free: s.free, black: s.black, pointer: s.pointer, transition: s.transition, started_at: startedAt }
  }
}
