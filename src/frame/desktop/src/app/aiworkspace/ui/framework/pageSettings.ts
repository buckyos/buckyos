/* "画布 ▸ 页面设置…" (UI 框架 PRD §4.1.2): the page settings of the canvas (its icon and page background) live in the
 * title zone's icon popover; the menu entry asks that popover to open. */

const listeners = new Set<() => void>()

export function onPageSettings(listener: () => void): () => void {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

export function requestPageSettings() {
  for (const listener of [...listeners]) listener()
}
