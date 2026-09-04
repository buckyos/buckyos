/**
 * Preview App — launch protocol between hosts, the window scheduler and the
 * app panel (PRD §13). Hosts call `openPreview()` (see `launch.ts`); the
 * scheduler turns a request into a `PreviewLaunchPayload` carried by a
 * desktop window; the panel renders it with the Preview Component.
 */

import type {
  ContentRef,
  PreviewFitMode,
  PreviewSessionContext,
  PreviewUIMode,
} from '../../components/preview/types'

export const PREVIEW_APP_ID = 'preview'
export const PREVIEW_LAUNCH_KIND = 'preview-launch'

export interface PreviewLaunchOrigin {
  /** Host app id (`files`, `messagehub`, …). */
  app?: string
  /** Host-side browsing context (a pane url, a chat id) — relevance only. */
  hostContext?: string
}

export interface PreviewOpenRequest {
  source: ContentRef
  session?: PreviewSessionContext
  uiMode?: PreviewUIMode
  fitMode?: PreviewFitMode
  origin?: PreviewLaunchOrigin
  /** Manual "open in new window": always a fresh, protected window (§13.3). */
  newWindow?: boolean
}

export interface PreviewLaunchPayload {
  kind: typeof PREVIEW_LAUNCH_KIND
  requestId: string
  source: ContentRef
  session?: PreviewSessionContext
  uiMode?: PreviewUIMode
  fitMode?: PreviewFitMode
  origin?: PreviewLaunchOrigin
  createdBy: 'auto' | 'manual'
}

export function isPreviewLaunchPayload(value: unknown): value is PreviewLaunchPayload {
  return (
    typeof value === 'object' &&
    value !== null &&
    (value as PreviewLaunchPayload).kind === PREVIEW_LAUNCH_KIND &&
    typeof (value as PreviewLaunchPayload).requestId === 'string' &&
    typeof (value as PreviewLaunchPayload).source === 'object'
  )
}
