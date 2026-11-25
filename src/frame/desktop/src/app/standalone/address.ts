/* ── The address of a system app's own page: `/app/<id>`, or the route it already had ──
 *
 * `OpenAppRequest.path` of a system app is appended to it: `/appearance` of Settings is `/app/settings/appearance`,
 * `/?taskid=1` of Task Center is `/taskcenter?taskid=1`. Names the app defines are path segments; everything else (a
 * folder, a DID) is a query parameter, because the Zone's static file server refuses some characters in a path.
 */

const ROUTES: Readonly<Record<string, string>> = {
  messagehub: '/messagehub',
  homestation: '/homestation',
  aiworkspace: '/workspace',
  'task-center': '/taskcenter',
}

const APP_ROUTE_PREFIX = '/app/'

export function systemAppAddress(appId: string, path?: string): string {
  const base = ROUTES[appId] ?? `${APP_ROUTE_PREFIX}${appId}`
  if (!path || path === '/') return base
  return /^\/[?#]/.test(path) ? `${base}${path.slice(1)}` : `${base}${path}`
}

/** The page inside an app that a location shows, as `OpenAppRequest.path`. */
export function systemAppPath(appId: string, location: Pick<Location, 'pathname' | 'search' | 'hash'>): string {
  const base = systemAppAddress(appId)
  const rest = location.pathname.startsWith(base) ? location.pathname.slice(base.length).replace(/\/+$/, '') : ''
  return `${rest || '/'}${location.search}${location.hash}`
}

/** A page path from a name and parameters; parameters without a value are left out. */
export function appPath(name: string, params: Record<string, string | null | undefined> = {}): string {
  const query = new URLSearchParams()
  for (const [key, value] of Object.entries(params)) if (value) query.set(key, value)
  return `/${name}${query.size ? `?${query}` : ''}`
}
