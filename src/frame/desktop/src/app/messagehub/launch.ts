import { z } from 'zod'
import { defaultContext } from './mock/store'

export const messageHubLaunchSchema = z.object({
  kind: z.literal('messagehub'),
  entityId: z.string().startsWith('did:').nullable(),
  context: z.object({ viewerDid: z.string().startsWith('did:'), ownerDid: z.string().startsWith('did:'), mode: z.enum(['self', 'observe']) }),
})
export function messageHubRouteContext(params: URLSearchParams) {
  if (!params.has('ownerDid') && !params.has('mode')) return defaultContext
  return { viewerDid: defaultContext.viewerDid, ownerDid: params.get('ownerDid') ?? '', mode: params.get('mode') === 'observe' ? 'observe' as const : 'self' as const }
}
