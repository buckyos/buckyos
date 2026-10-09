import { z } from 'zod'
import type { MessageHubContext } from './types'

export const messageHubLaunchSchema = z.object({
  kind: z.literal('messagehub'),
  entityId: z.string().startsWith('did:').nullable(),
  /** Opens this session of the entity instead of its default one. */
  sessionId: z.string().min(1).nullable().optional(),
  /** Locates this message (the ObjId of its anchor) in the session and opens its details. */
  messageId: z.string().min(1).nullable().optional(),
  context: z.object({ viewerDid: z.string().startsWith('did:'), ownerDid: z.string().startsWith('did:'), mode: z.enum(['self', 'observe']) }),
})

/** Partial context from a route / launch; the viewer is always the logged-in user. */
export interface MessageHubContextRequest {
  ownerDid?: string
  mode?: 'self' | 'observe'
}

export function messageHubRouteContext(params: URLSearchParams): MessageHubContextRequest | undefined {
  if (!params.has('ownerDid') && !params.has('mode')) return undefined
  return { ownerDid: params.get('ownerDid') ?? '', mode: params.get('mode') === 'observe' ? 'observe' : 'self' }
}

export function resolveMessageHubContext(base: MessageHubContext, request?: MessageHubContextRequest | MessageHubContext): MessageHubContext {
  if (!request) return base
  const mode = request.mode ?? 'self'
  const ownerDid = mode === 'observe' ? request.ownerDid ?? '' : base.ownerDid
  return { viewerDid: base.viewerDid, ownerDid, mode }
}

/** Address of one message: its session plus the ObjId of the anchor message, which edits never change. */
export function messageHubMessagePath(target: { entityId: string; sessionId: string; messageId: string }, context: MessageHubContext): string {
  const params = new URLSearchParams({ entityId: target.entityId, sessionId: target.sessionId, messageId: target.messageId })
  if (context.mode === 'observe') { params.set('ownerDid', context.ownerDid); params.set('mode', 'observe') }
  return `/messagehub?${params.toString()}`
}
