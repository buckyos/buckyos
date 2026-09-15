import { normalizeViewportProgress } from '../models/layout'

/**
 * Launcher paging progress (0..1) is published as a CSS custom property on
 * the root element rather than through React state: Swiper reports it on
 * every animation frame while a page is being swiped, and routing it through
 * the store used to re-render the whole desktop route (and the background)
 * once per frame. The panorama wallpaper reads the variable in a `calc()`.
 */
export const DESKTOP_VIEWPORT_PROGRESS_VAR = '--desktop-viewport-progress'

export function setDesktopViewportProgress(progress: number, pageCount: number) {
  document.documentElement.style.setProperty(
    DESKTOP_VIEWPORT_PROGRESS_VAR,
    normalizeViewportProgress(progress, pageCount).toFixed(4),
  )
}

export function resetDesktopViewportProgress() {
  document.documentElement.style.removeProperty(DESKTOP_VIEWPORT_PROGRESS_VAR)
}
