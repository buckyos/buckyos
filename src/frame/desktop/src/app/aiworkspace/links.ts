/* Addresses of the AI Workspace (内核架构改进 D21 / T4-8, notepads/aiworkspace-subdomain-todo.md).
 *
 * A Workspace is a site of its own: `https://aiws-<label>.<zonehost>`, the label being the id `ws_<YYMM><4 base36>`
 * without its `ws_` prefix (`aiws-2611a0b3.<zone>`; a DNS label carries no `_`), served by the aiworkspace service
 * from the Desktop build's `aiws.html` entry. Its pages are
 * `/<workspace slug>/<surface slug>?id=<surface id>[&block=<block id>]`: the slugs are for people and crawlers, the
 * `id` decides (the service sends a Surface named without its id to this canonical form). The origin comes from
 * the service (`ws.get_info.public_base`); a deployment without a host (standalone backend, e2e) has none and the
 * Zone-host list page is all there is.
 *
 * On the Zone host only the list remains (`/workspace`, the phone's entry): opening a workspace from it goes to the
 * workspace's own origin — through the Control Panel's launch when the Desktop user is signed in (launch.ts), so
 * the new origin gets its own session (hybrid SSO: the host-only cookies `/sso_callback` sets there). */

import type { LaunchTarget } from './ui/shell/WorkspaceShell'

/** The signed-in user's workspace list on the Zone host. */
export const WORKSPACE_ROUTE = '/workspace'

const HOST_LABEL = /^aiws-([a-z0-9]+)\./

/** The service's `pub_name`: lower-cased, every run of non-ASCII-alphanumerics becomes one `-`, at most 64 characters. */
export function pubName(text: string): string {
  return text.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 64)
}

/** First path segments the Zone gateway routes elsewhere on every host, or the Desktop build's own directories: the
 * service's `RESERVED_WORKSPACE_SLUGS`. */
const RESERVED_SLUGS = new Set(['home', 'kapi', 'api', 's3', 'bucket', 'ndm', 'nfs', 'assets', 'aiworkspace', 'messagehub'])

export function workspaceSlug(title: string): string {
  const slug = pubName(title)
  return !slug || RESERVED_SLUGS.has(slug) ? 'workspace' : slug
}

export function surfaceSlug(title: string): string {
  return pubName(title) || 'surface'
}

/** True on a Workspace's own origin. */
export function onWorkspaceHost(host = window.location.host): boolean {
  return HOST_LABEL.test(host)
}

/** The workspace id a Workspace host names, null elsewhere. */
export function workspaceIdOfHost(host = window.location.host): string | null {
  const m = HOST_LABEL.exec(host)
  return m ? `ws_${m[1]}` : null
}

/** The Zone host's origin: where the Desktop, `/login` and `/sso/entry` live — this origin, or the parent of a
 * Workspace host (`aiws-<id>.<zone>` → `<zone>`). */
export function zoneOrigin(): string {
  const { protocol, host } = window.location
  return `${protocol}//${host.replace(HOST_LABEL, '')}`
}

export interface PageSurface { id: string; title: string }

/** What a link may point at inside a Block (控件机制 TODO G12): a record a data control shows (`record`), one occurrence
 * of a repeating event (`occ`: its original start), and the control as a page of its own (`page=control`, G14). */
export interface RecordTarget { record?: string | null; occurrence?: string | null; page?: 'control' | null }

/** A Workspace's page: `<public_base>/<workspace slug>[/<surface slug>?id=<surface id>[&block=][&record=][&occ=][&page=]]`;
 * null without a public base (then there is no page to link to). */
export function workspacePageUrl(publicBase: string | null | undefined, workspaceTitle: string, surface?: PageSurface | null, blockId?: string | null, inner?: RecordTarget): string | null {
  if (!publicBase) return null
  const base = publicBase.replace(/\/+$/, '')
  const ws = workspaceSlug(workspaceTitle)
  if (!surface) return `${base}/${ws}`
  return `${base}/${ws}/${surfaceSlug(surface.title)}?${targetQuery(surface.id, blockId, inner)}`
}

/** The query naming a Surface, a Block in it and what inside the Block. */
export function targetQuery(surfaceId: string, blockId?: string | null, inner?: RecordTarget): URLSearchParams {
  const query = new URLSearchParams({ id: surfaceId })
  if (blockId) query.set('block', blockId)
  if (inner?.record) query.set('record', inner.record)
  if (inner?.occurrence) query.set('occ', inner.occurrence)
  if (inner?.page) query.set('page', inner.page)
  return query
}

/** `https://aiws-<label>.<zoneHost>`: a Workspace's origin from a Zone host, when no service told it (Settings lists). */
export function workspaceOriginOn(zoneHost: string, workspaceId: string): string {
  return `https://aiws-${workspaceId.replace(/^ws_/, '')}.${zoneHost}`
}

/** What a page's address asks to open: `id` names the Surface, `block` the Block in it, `record` / `occ` a record (an
 * occurrence) the Block shows, `page=control` the Block as its own page. */
export function targetOf(search: URLSearchParams): LaunchTarget | null {
  const surfaceId = search.get('id')
  const blockId = search.get('block')
  if (!surfaceId && !blockId) return null
  return { surfaceId, blockId, record: search.get('record'), occurrence: search.get('occ'), page: search.get('page') === 'control' ? 'control' : null }
}

/** The Desktop login on the Zone host, coming back to the current address afterwards (a guest who wants to sign in, a
 * private link). On a Workspace host, the Zone host's login page then signs the visitor into this origin. */
export function loginUrl(): string {
  const url = new URL('/login', zoneOrigin())
  url.searchParams.set('redirect_url', window.location.href)
  return url.toString()
}

/** The Zone's entry page for a share entry (hybrid SSO contract §10): it signs the visitor in and joins the entry. */
export function entryUrl(entryId: string): string {
  const url = new URL('/sso/entry', zoneOrigin())
  url.searchParams.set('service', 'aiworkspace')
  url.searchParams.set('entry', entryId)
  return url.toString()
}
