/**
 * Host entry points into the Preview App (PRD §13.2).
 *
 *   openPreview({ source, session, origin })            → Smart / Single Window policy
 *   openPreview({ ..., newWindow: true })               → always a fresh, protected window
 *
 * Hosts pass a Source, an optional Session Context and where the request came
 * from; they never choose or manage Preview windows themselves (§6.7).
 */

import { previewWindowManager } from './windowManager'
import type { PreviewOpenRequest } from './types'

export type { PreviewOpenRequest } from './types'

export function openPreview(request: PreviewOpenRequest): string | null {
  return previewWindowManager.open(request)
}

export function openPreviewInNewWindow(request: Omit<PreviewOpenRequest, 'newWindow'>): string | null {
  return previewWindowManager.open({ ...request, newWindow: true })
}
