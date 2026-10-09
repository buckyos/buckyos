import type { LaunchTarget } from './ui/shell/WorkspaceShell'

export const WORKSPACE_ROUTE = '/workspace'

export function workspacePath(workspaceId: string | null, target?: LaunchTarget | null): string {
  if (!workspaceId) return WORKSPACE_ROUTE
  const query = new URLSearchParams()
  if (target?.surfaceId) query.set('surface', target.surfaceId)
  if (target?.blockId) query.set('block', target.blockId)
  const search = query.toString()
  return `${WORKSPACE_ROUTE}/${encodeURIComponent(workspaceId)}${search ? `?${search}` : ''}`
}

export function workspaceUrl(workspaceId: string | null, target?: LaunchTarget | null): string {
  return new URL(workspacePath(workspaceId, target), window.location.origin).toString()
}

export function targetOf(search: URLSearchParams): LaunchTarget | null {
  const surfaceId = search.get('surface')
  const blockId = search.get('block')
  return surfaceId || blockId ? { surfaceId, blockId } : null
}
