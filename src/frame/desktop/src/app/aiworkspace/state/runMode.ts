/* Design and run mode of a canvas (扩展控件 P2, Q27): a local, unpersisted switch (▶ / ✎) per canvas sub-mode, never in
 * the user work state. Defaults follow the mode (`modePolicy(mode).events`): edit designs, view runs, a show runs only
 * the Blocks marked operable on stage, path editing never runs; replays, guests and phones are off and locked. The
 * stage and the canvas hand the result down through `RunModeContext`; without a provider nothing runs. */

import { createContext, useSyncExternalStore } from 'react'
import { modePolicy, type CanvasMode } from '../ui/blocks/registry'
import { Emitter } from './emitter'

export class RunModeState {
  private readonly choice = new Map<CanvasMode, boolean>()
  private readonly emitter = new Emitter()
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = (): number => this.version
  chosen(mode: CanvasMode): boolean | undefined { return this.choice.get(mode) }
  set(mode: CanvasMode, on: boolean) {
    this.choice.set(mode, on)
    this.version += 1
    this.emitter.emit()
  }
}

export interface RunMode { interactive: boolean; locked: boolean; set: (on: boolean) => void }

/** The run mode of `mode` for this window: what was switched, else the mode's default; `lockedOff` (a guest, a phone)
 * keeps it off. */
export function useRunMode(state: RunModeState, mode: CanvasMode, lockedOff: boolean): RunMode {
  useSyncExternalStore(state.subscribe, state.snapshot)
  const events = modePolicy(mode).events
  const locked = lockedOff || events === 'off' || events === 'live'
  const interactive = lockedOff || events === 'off' ? false : events === 'live' ? true : state.chosen(mode) ?? events === 'run'
  return { interactive, locked, set: (on) => { if (!locked) state.set(mode, on) } }
}

export const RunModeContext = createContext<{ interactive: boolean }>({ interactive: false })
