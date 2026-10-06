/* Hooks for the e2e suite, present only while the dev override of transport.ts is active
 * (`localStorage['aiworkspace.dev']`). They expose state the DOM cannot show: the editor document JSON. */

import type { RichTextAnchorHost, RichTextHit } from '../anchors/richtext'
import type { AnchorRegistry } from '../anchors/registry'
import { DEV_OVERRIDE_KEY } from './transport'

interface TestHooks {
  editors: Record<string, () => unknown>
  canonicalize?: (ast: unknown) => unknown
  /** The rich text anchor registry, so a test can play an application registering its own range kind. */
  richTextAnchors?: AnchorRegistry<RichTextAnchorHost, RichTextHit>
  /** Fault injection into the Replica Worker of the open workspace (replica session only). */
  replica?: { failTransactions(kind: 'quota' | 'error', count: number): Promise<void>; killWorker(): void }
  /** The render probe reads what the canvas mounted (phase two §9.5). */
  canvas?: { surfaceId: string; blocks: number; mounted: number; hidden: number; placeholders: number; editors: number; html: number; zoom: number; mode: string }
  /** Commit counter of the open workspace (gesture = one commit). */
  commits?: number
  /** The outline model's current entities (incremental structure, phase two §9.3). */
  outline?: () => unknown[]
}

declare global {
  interface Window { __aiwsTestHooks?: TestHooks }
}

function active(): boolean {
  try { return window.localStorage.getItem(DEV_OVERRIDE_KEY) !== null } catch { return false }
}

export function testHooks(): TestHooks | null {
  if (!active()) return null
  window.__aiwsTestHooks ??= { editors: {} }
  return window.__aiwsTestHooks
}
