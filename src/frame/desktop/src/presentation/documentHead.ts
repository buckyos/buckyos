/* ── Browser tab title and favicon ──
 *
 * Pages declare their own title; the Zone provides the brand name and icon.
 * The tab reads "Page · Zone" (or just "Zone"), and the favicon is the Zone icon when one is set.
 * Titles are a stack, so leaving a page restores whatever is underneath instead of a stale string.
 */

import { useEffect } from 'react'

const DEFAULT_TITLE = 'BuckyOS'

interface TitleEntry {
  title: string
}

let titles: TitleEntry[] = []
let brand: { title?: string; icon?: string } = {}
let defaultIcon: { href: string; type: string | null } | null = null

export function composeDocumentTitle(pageTitle: string | undefined, zoneTitle: string | undefined): string {
  if (pageTitle && zoneTitle && pageTitle !== zoneTitle) return `${pageTitle} · ${zoneTitle}`
  return pageTitle || zoneTitle || DEFAULT_TITLE
}

function imageMimeType(url: string): string | null {
  return /^data:(image\/[a-z0-9.+-]+)[;,]/i.exec(url)?.[1] ?? null
}

function applyDocumentHead() {
  document.title = composeDocumentTitle(titles.at(-1)?.title, brand.title)
  const link = document.querySelector<HTMLLinkElement>('link[rel~="icon"]')
  if (!link) return
  defaultIcon ??= { href: link.getAttribute('href') ?? '/favicon.svg', type: link.getAttribute('type') }
  const href = brand.icon ?? defaultIcon.href
  const type = brand.icon ? imageMimeType(brand.icon) : defaultIcon.type
  if (link.getAttribute('href') !== href) link.setAttribute('href', href)
  if (type) link.setAttribute('type', type)
  else link.removeAttribute('type')
}

/** Shows `title` in the tab while the calling component is mounted. */
export function useDocumentTitle(title: string | null | undefined) {
  useEffect(() => {
    if (!title) return
    const entry: TitleEntry = { title }
    titles = [...titles, entry]
    applyDocumentHead()
    return () => {
      titles = titles.filter((item) => item !== entry)
      applyDocumentHead()
    }
  }, [title])
}

export function useDocumentBrand(title: string | undefined, icon: string | undefined) {
  useEffect(() => {
    brand = { title, icon }
    applyDocumentHead()
  }, [title, icon])
}
