/* Hooks for the e2e suite, present only while the dev override of transport.ts is active
 * (`localStorage['aiworkspace.dev']`). They expose state the DOM cannot show: the editor document JSON. */

import type { RichTextAnchorHost, RichTextHit } from '../anchors/richtext'
import type { AnchorRegistry } from '../anchors/registry'
import type { BlockRegistry } from '../ui/blocks/registry'
import { DEV_OVERRIDE_KEY } from './transport'

interface TestHooks {
  editors: Record<string, () => unknown>
  canonicalize?: (ast: unknown) => unknown
  /** The rich text anchor registry, so a test can play an application registering its own range kind. */
  richTextAnchors?: AnchorRegistry<RichTextAnchorHost, RichTextHit>
  /** The Block registry, so a test can play an extension registering a Block (e.g. one with its own line anchors). */
  blocks?: BlockRegistry
  /** Fault injection into the Replica Worker of the open workspace (replica session only). */
  replica?: { failTransactions(kind: 'quota' | 'error', count: number): Promise<void>; killWorker(): void }
  /** The render probe reads what the canvas mounted (phase two §9.5). */
  canvas?: { surfaceId: string; blocks: number; mounted: number; hidden: number; placeholders: number; editors: number; html: number; zoom: number; mode: string; ink?: { busy: boolean; waiting: number; erasing: number } }
  /** Commit counter of the open workspace (gesture = one commit). */
  commits?: number
  /** The outline model's current entities (incremental structure, phase two §9.3). */
  outline?: () => unknown[]
  /** Send Commits in parts of this many bytes (a small import then goes through `doc.stage` like a large one). */
  commitPartBytes?: number
  /** A show's stage (第三期规划 §9): the camera of the Surface on stage, and the transitions it ran with their timings. */
  stage?: { view: () => { x: number; y: number; zoom: number; surfaceId: string | null }; transitions: { kind: string; step: string; startedAt: number; arrivedAt: number | null; frames: number[] }[] }
  /** Presence and the work log of the open workspace (UI 框架 PRD K2, K3), as the hooks use them. */
  activity?: {
    /** `usePresence` without a component: `retain` registers this window, `release` leaves; `snapshot` is what the hook returns. */
    presence: { retain(): void; release(): void; beat(): Promise<void>; snapshot(): unknown; surface(): string | null; select(surfaceId: string): void; labels(): { name: string; group: string | null }[] }
    /** `useWorkLog` without a component: a feed per filter, kept on the hooks (`refresh`, `loadMore`, `snapshot`). */
    workLog(filter: 'all' | 'mine' | 'automation', limit?: number): { refresh(): Promise<void>; loadMore(): Promise<void>; snapshot(): unknown; describe(): string[] }
  }
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
