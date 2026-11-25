import { z } from 'zod'
import type { MessageHubContext } from './types'

/** A share handed over by the system share dialog: an attachment (`objId`) or a link (`url`). */
export const sharedLaunchSchema = z.object({
  shareId: z.string().min(1),
  objId: z.string().min(1).optional(),
  name: z.string(),
  size: z.number().optional(),
  mime: z.string().nullable().optional(),
  url: z.string().url().nullable().optional(),
  title: z.string().optional(),
})

export type SharedLaunch = z.infer<typeof sharedLaunchSchema>

export const messageHubLaunchSchema = z.object({
  kind: z.literal('messagehub'),
  entityId: z.string().startsWith('did:').nullable(),
  /** Opens this session of the entity instead of its default one. */
  sessionId: z.string().min(1).nullable().optional(),
  /** Locates this message (the ObjId of its anchor) in the session and opens its details. */
  messageId: z.string().min(1).nullable().optional(),
  context: z.object({ viewerDid: z.string().startsWith('did:'), ownerDid: z.string().startsWith('did:'), mode: z.enum(['self', 'observe']) }),
  share: sharedLaunchSchema.optional(),
})

export const MESSAGE_HUB_LINK_KIND = 'messagehub-link'

/** A launch that is an address of MessageHub: the query of `/messagehub?…`. */
export const messageHubLinkSchema = z.object({ kind: z.literal(MESSAGE_HUB_LINK_KIND), search: z.string().max(4096) })

/** The address of a launch (the reverse of reading `/messagehub?…`). */
export function messageHubLinkParams(launch: z.infer<typeof messageHubLaunchSchema>): URLSearchParams {
  const params = new URLSearchParams()
  const observe = launch.context.mode === 'observe'
  for (const [key, value] of Object.entries({ entityId: launch.entityId, sessionId: launch.sessionId, messageId: launch.messageId, ownerDid: observe ? launch.context.ownerDid : null, mode: observe ? 'observe' : null })) {
    if (value) params.set(key, value)
  }
  if (launch.share) params.set('share', JSON.stringify(launch.share))
  return params
}

export function messageHubLinkShare(params: URLSearchParams): SharedLaunch | undefined {
  try { return sharedLaunchSchema.parse(JSON.parse(params.get('share') ?? '')) } catch { return undefined }
}

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
