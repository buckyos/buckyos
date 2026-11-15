/* The viewport the service is asked for (D19): it follows the device that is looking; "Desktop site" asks for a
 * desktop layout instead, which a narrow screen then shows scaled to its width. */

import { VIEWPORT_MAX, VIEWPORT_MIN, type Viewport } from '../api/types'

export const MOBILE_MAX_WIDTH = 768
export const DESKTOP_SITE_WIDTH = 1280

export interface Size { width: number; height: number }

const clamp = (value: number) => Math.max(VIEWPORT_MIN, Math.min(VIEWPORT_MAX, Math.round(value)))

export function viewportFor(size: Size, desktopSite: boolean): Viewport {
  if (desktopSite && size.width < DESKTOP_SITE_WIDTH) {
    return { width: DESKTOP_SITE_WIDTH, height: clamp((DESKTOP_SITE_WIDTH * size.height) / Math.max(1, size.width)), mobile: false }
  }
  return { width: clamp(size.width), height: clamp(size.height), mobile: !desktopSite && size.width <= MOBILE_MAX_WIDTH }
}
