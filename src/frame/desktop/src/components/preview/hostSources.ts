import type { ContentRef, ResolvedPreviewSource } from './types'

export interface PreviewSourceResolver {
  resolvePreviewSource(
    source: ContentRef,
    opts: { signal?: AbortSignal; permissionsContext?: unknown },
  ): Promise<ResolvedPreviewSource>
}

const resolvers = new Map<string, PreviewSourceResolver>()

export function registerPreviewSourceResolver(kind: string, resolver: PreviewSourceResolver): () => void {
  resolvers.set(kind, resolver)
  return () => {
    if (resolvers.get(kind) === resolver) resolvers.delete(kind)
  }
}

export function previewSourceResolverFor(kind: string): PreviewSourceResolver | undefined {
  return resolvers.get(kind)
}
