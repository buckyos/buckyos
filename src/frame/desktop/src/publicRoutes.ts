// Public (login-optional) routes.
//
// By default `bootstrap()` in main.tsx forces a redirect to /login whenever
// there is no signed-in account. Some internal pages are meant to be reachable
// without that gate (e.g. a shareable profile page that renders a given user's
// public info). List those paths here so the login redirect is skipped for
// them. The page itself is still responsible for working in a logged-out state.

export const PUBLIC_ROUTE_PREFIXES = ['/login', '/userprofile'] as const

const normalizePath = (pathname: string) =>
  (pathname || '/').replace(/\/+$/, '') || '/'

/**
 * Returns true when `pathname` belongs to a login-optional route, matching the
 * exact path or any nested sub-path (e.g. `/userprofile/abc`).
 */
export function isPublicRoute(pathname: string): boolean {
  const normalized = normalizePath(pathname)
  return PUBLIC_ROUTE_PREFIXES.some(
    (prefix) => normalized === prefix || normalized.startsWith(`${prefix}/`),
  )
}
