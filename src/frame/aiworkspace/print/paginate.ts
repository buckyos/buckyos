/* Flow pagination (打印 TODO §3.2, T2, Q18): where the pages of a flow canvas end. A pure function of measured heights
 * — the print renderer measures the Blocks in Chromium at the content width and lays the pages out from the result —
 * so the rules are tested here:
 *
 *   - a page ends only at a legal break: between Blocks, or at a break a Block declares (between paragraphs or lines of
 *     a rich text, between the rows of a table);
 *   - `keepWithNext` keeps a Block on the page of what follows it (a heading never ends a page);
 *   - a table's header (`header`) is repeated at the top of each page it continues on;
 *   - what cannot be split and is taller than a page is scaled down to one page (and reported); nothing is cut off.
 *
 * Units are whatever the caller measured in (CSS pixels); the content height is already less the header, footer and
 * margins of the page. */

export interface FlowItem {
  id: string
  height: number
  /** Offsets from the item's top where a page may end inside it, ascending, each in (0, height). Empty: unbreakable. */
  breaks?: number[]
  /** Keep on the page of the next item. */
  keepWithNext?: boolean
  /** Height of a header repeated at the top of every page the item continues on (a table's header row). */
  header?: number
  /** Space after the item, dropped when a page ends there. */
  gapAfter?: number
}

/** A part of an item on a page: `[from, to)` of its height, at `top` of the content area. `header`: the repeated header
 * is drawn first (the part starts below it); `scale` < 1: drawn smaller to fit the page. */
export interface Fragment {
  item: number
  id: string
  from: number
  to: number
  top: number
  header?: number
  scale?: number
}

export interface FlowPage {
  fragments: Fragment[]
}

export interface PaginateIssue {
  kind: 'shrunk'
  id: string
  scale: number
}

export interface Pagination {
  pages: FlowPage[]
  issues: PaginateIssue[]
}

const EPS = 0.5

export function paginate(items: FlowItem[], contentHeight: number): Pagination {
  const H = Math.max(1, contentHeight)
  const pages: FlowPage[] = []
  const issues: PaginateIssue[] = []
  let page: FlowPage = { fragments: [] }
  let y = 0
  const newPage = () => {
    if (page.fragments.length) pages.push(page)
    page = { fragments: [] }
    y = 0
  }
  const place = (f: Omit<Fragment, 'top'>, drawn: number) => {
    page.fragments.push({ ...f, top: y })
    y += drawn
  }
  const firstChunk = (it: FlowItem) => {
    const b = (it.breaks ?? []).find((v) => v > EPS && v < it.height - EPS)
    return b ?? it.height
  }

  items.forEach((it, index) => {
    const height = Math.max(0, it.height)
    const gap = Math.max(0, it.gapAfter ?? 0)
    const breaks = (it.breaks ?? []).filter((b) => b > EPS && b < height - EPS).sort((a, b) => a - b)
    const header = Math.max(0, it.header ?? 0)
    const next = items[index + 1]
    // a Block kept with the next one moves on with it when the next one's first part would not fit after it
    if (it.keepWithNext && next && y > 0 && height <= H - y + EPS) {
      const after = y + height + gap
      if (after + Math.min(firstChunk(next), H) > H + EPS && height + gap + Math.min(firstChunk(next), H) <= H + EPS) newPage()
    }
    if (height <= H - y + EPS) {
      place({ item: index, id: it.id, from: 0, to: height }, height)
      y += gap
      if (y > H - EPS) newPage()
      return
    }
    if (breaks.length === 0) {
      if (y > 0) newPage()
      if (height <= H + EPS) {
        place({ item: index, id: it.id, from: 0, to: height }, height)
        y += gap
        if (y > H - EPS) newPage()
        return
      }
      const scale = H / height
      issues.push({ kind: 'shrunk', id: it.id, scale })
      place({ item: index, id: it.id, from: 0, to: height, scale }, H)
      newPage()
      return
    }
    let start = 0
    while (start < height - EPS) {
      const repeat = start > 0 ? Math.min(header, H / 2) : 0
      const avail = H - y - repeat
      if (height - start <= avail + EPS) {
        place({ item: index, id: it.id, from: start, to: height, ...(repeat ? { header: repeat } : {}) }, repeat + height - start)
        y += gap
        if (y > H - EPS) newPage()
        break
      }
      let cut = -1
      for (const b of breaks) {
        if (b <= start + EPS) continue
        if (b - start <= avail + EPS) cut = b
        else break
      }
      if (cut < 0) {
        if (y > 0) { newPage(); continue }
        // not one legal break within a whole page: the next part, scaled down to the page
        const end = breaks.find((b) => b > start + EPS) ?? height
        const full = H - repeat
        const scale = full / (end - start)
        issues.push({ kind: 'shrunk', id: it.id, scale })
        place({ item: index, id: it.id, from: start, to: end, scale, ...(repeat ? { header: repeat } : {}) }, H)
        newPage()
        start = end
        continue
      }
      place({ item: index, id: it.id, from: start, to: cut, ...(repeat ? { header: repeat } : {}) }, repeat + cut - start)
      newPage()
      start = cut
    }
  })
  if (page.fragments.length) pages.push(page)
  if (pages.length === 0) pages.push({ fragments: [] })
  return { pages, issues }
}
