// HomeStation URLs (architecture §4.4/§4.6):
//
//   https://<zone>/homestation                 the signed-in user's own HomeStation
//   https://<zone>/homestation/<user>          that user's home feed (public portal)
//   https://<zone>/homestation/<user>/<key>    one post of that user (`…/feed/@/<key>` entry)
//   https://<zone>/homestation/~zone           the zone feed
//   https://www.<zone>/, https://homestation.<zone>/   the zone's default feed
//
// Only `/` on the two short hosts is special; every link a portal page renders points at the
// zone host's `/homestation/...`.

export const ZONE_FEED = '~zone'

const SHORT_HOST_PREFIXES = ['www.', 'homestation.']

function shortHostPrefix(host: string) {
  return SHORT_HOST_PREFIXES.find(prefix => host.toLowerCase().startsWith(prefix))
}

/** `www.<zone>/` or `homestation.<zone>/`: opens the zone's default feed. */
export function isPortalRoot(location: Pick<Location, 'host' | 'pathname'> = window.location) {
  return Boolean(shortHostPrefix(location.host)) && (location.pathname === '/' || location.pathname === '')
}

/** Origin of the zone host (`https://<zone>`), also when the page is on a short host. */
export function zoneOrigin(location: Pick<Location, 'protocol' | 'host'> = window.location) {
  const prefix = shortHostPrefix(location.host)
  return `${location.protocol}//${prefix ? location.host.slice(prefix.length) : location.host}`
}

export function portalPath(feed: string, key?: string) {
  return `/homestation/${encodeURIComponent(feed)}${key ? `/${encodeURIComponent(key)}` : ''}`
}

/** Link to a portal page: same-origin path on the zone host, absolute from a short host. */
export function portalHref(feed: string, key?: string) {
  const path = portalPath(feed, key)
  return shortHostPrefix(window.location.host) ? `${zoneOrigin()}${path}` : path
}

/** Absolute URL to share. */
export function portalShareUrl(feed: string, key?: string) {
  return `${zoneOrigin()}${portalPath(feed, key)}`
}

/** `cyfs://<zone>/home/<user>/<ns>/@/<key>` → `{ user, key }` (feed namespace only). */
export function parseEntry(entry: string | undefined): { user: string; key: string } | null {
  const match = entry?.match(/^cyfs:\/\/[^/]+\/home\/([^/]+)\/feed\/@\/([^/]+)$/)
  return match ? { user: decodeURIComponent(match[1]), key: decodeURIComponent(match[2]) } : null
}
