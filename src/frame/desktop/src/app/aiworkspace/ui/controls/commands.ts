/* Requests a Block inside a control sends to the canvas that owns selection and editing (脑图方案 §7.1): a node's
 * editor saving with Tab asks for a child, a toolbar button for a sibling. The canvas performs them (one commit) and
 * selects and edits the new Block. */

import { Emitter } from '../../state/emitter'

export type ControlCommand = { kind: 'add-child' | 'add-sibling'; nodeId: string } | { kind: 'select'; ids: string[]; edit?: string }

class ControlCommands {
  private readonly emitter = new Emitter()
  private pending: ControlCommand[] = []
  readonly subscribe = this.emitter.subscribe

  send(command: ControlCommand) {
    this.pending.push(command)
    this.emitter.emit()
  }

  /** The requests not yet performed (the receiver takes them all). */
  take(): ControlCommand[] {
    const out = this.pending
    this.pending = []
    return out
  }
}

export const controlCommands = new ControlCommands()
