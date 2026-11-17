/* The assets of a print package (打印 TODO T7): files next to the renderer, by object id (`PrintPackage.assets`). A page
 * shows them by their relative address; nothing reads their bytes (Chromium fetches no local file over file://). Outside
 * a print package there is no such address. */

import type { PrintPackage } from '@print/package.ts'

export function printPackage(): PrintPackage | null {
  return typeof window === 'undefined' ? null : window.__AIWS_PRINT__ ?? null
}

export function printAssetUrl(objectId: string | null | undefined): string | null {
  return objectId ? printPackage()?.assets?.[objectId] ?? null : null
}
