import type { ReactNode } from 'react'
import type { MessageObject } from '../model/msgobj'
import type { MessageRenderContext } from './bubble'

export type MessageRenderer = (message: MessageObject, context: MessageRenderContext) => ReactNode | null

/**
 * Where a host's renderer joins the chain: `notice` before redactions (a card that stands for an operation, such as
 * an invitation), `card` after them (a card that shows a record, such as a notification). Without a renderer the
 * message shows as an ordinary one.
 */
export type MessageRendererSlot = 'notice' | 'card'

const slots: Record<MessageRendererSlot, MessageRenderer[]> = { notice: [], card: [] }

export function registerMessageRenderer(slot: MessageRendererSlot, renderer: MessageRenderer): void {
  if (!slots[slot].includes(renderer)) slots[slot].push(renderer)
}

export function registeredRenderers(slot: MessageRendererSlot): readonly MessageRenderer[] {
  return slots[slot]
}
