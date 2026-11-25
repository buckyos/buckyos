/* The bookmarks file a browser exports (an HTML page of links): the websites in it, one per host. */

export interface BookmarkSite {
  host: string
  /** The bookmark itself when the host has one; the host's front page when it has several. */
  url: string
  title: string
  /** How many bookmarks of the file are on this host. */
  count: number
}

export function bookmarkSites(html: string): BookmarkSite[] {
  const sites = new Map<string, BookmarkSite>()
  for (const link of new DOMParser().parseFromString(html, 'text/html').querySelectorAll('a[href]')) {
    let url: URL
    try {
      url = new URL(link.getAttribute('href') ?? '')
    } catch {
      continue
    }
    if (url.protocol !== 'http:' && url.protocol !== 'https:') continue
    const host = url.hostname.replace(/\.$/, '')
    if (!/^[a-z0-9._-]+$/.test(host) || !host.includes('.')) continue
    const known = sites.get(host)
    if (known) {
      known.count += 1
      known.url = `${url.origin}/`
      known.title = host
    } else {
      url.hash = ''
      sites.set(host, { host, url: url.href, title: link.textContent?.replace(/\s+/g, ' ').trim() || host, count: 1 })
    }
  }
  return [...sites.values()]
}
