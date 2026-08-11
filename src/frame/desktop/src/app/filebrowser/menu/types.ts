/**
 * File browser context-menu model.
 *
 * Menus are described as pure data — no React, no platform assumptions.
 * Hosts build a `FileMenuContext` for an invocation (an item, a multi-item
 * selection, or the blank view area), ask the registry for sections, and
 * render them with whatever interaction fits the platform: a right-click
 * popup on desktop, an action sheet on mobile.
 *
 * This is the system-level extension point of the file manager: extra
 * providers can be registered at runtime, and a `FileMenuConfig` overlay can
 * hide or reorder what the defaults contribute.
 */

import type { FileEntry, SortKey, ViewMode } from '../types'
import type { FileItem, LocationCapabilities } from '../data/FolderReader'

/** What the menu was invoked on. */
export type FileMenuTarget = 'item' | 'selection' | 'view'

/** Renderer resolves `key` through i18n, falling back to `fallback`. */
export interface FileMenuLabel {
  key: string
  fallback: string
  vars?: Record<string, string | number>
}

/** Renderer-agnostic icon id, mapped to an icon set per platform. */
export type FileMenuIcon =
  | 'open'
  | 'open-new-tab'
  | 'open-right'
  | 'preview'
  | 'copy'
  | 'link'
  | 'share'
  | 'download'
  | 'rename'
  | 'move'
  | 'trash'
  | 'new-folder'
  | 'upload'
  | 'refresh'
  | 'select-all'
  | 'view-list'
  | 'view-icon'
  | 'collection'
  | 'remove-ref'
  | 'jump'
  | 'broken'
  | 'move-up'
  | 'move-down'

export interface FileMenuAction {
  type: 'action'
  /** Unique within the built menu; config hides entries by this id. */
  id: string
  /** Command dispatched to the host when the entry is picked. */
  command: string
  /** Optional command payload (e.g. a move-target path). */
  args?: Record<string, unknown>
  label: FileMenuLabel
  icon?: FileMenuIcon
  danger?: boolean
  disabled?: boolean
  /** Display-only hint, e.g. '⌘C'. */
  shortcut?: string
}

export interface FileMenuSubmenu {
  type: 'submenu'
  id: string
  label: FileMenuLabel
  icon?: FileMenuIcon
  items: FileMenuAction[]
}

export type FileMenuEntryItem = FileMenuAction | FileMenuSubmenu

/** One visually-grouped section; renderers separate sections with dividers. */
export type FileMenuSection = FileMenuEntryItem[]

export interface FileMenuContext {
  target: FileMenuTarget
  /** Exactly 1 item for 'item', 2+ for 'selection', 0 for 'view'. */
  items: FileItem[]
  /** Convenience projection of `items` — same order. */
  entries: FileEntry[]
  /** Canonical location url of the invoking pane. */
  currentUrl: string
  viewMode: ViewMode
  /** Location capabilities — providers trim themselves by these, never by scheme. */
  capabilities: LocationCapabilities
  /** Active sort key (manual ordering actions only make sense under 'manual'). */
  sortKey: SortKey
  /** Existing collections for the "Add to Collection" submenu. */
  collections: { id: string; title: string }[]
  /** Folders offered as quick "Move to" targets. */
  moveTargets: { label: string; path: string }[]
  pane: {
    canOpenInNewTab: boolean
    canOpenInRightPane: boolean
  }
}

export interface FileMenuProvider {
  id: string
  /** Sections from lower-order providers render first. */
  order: number
  /** Skip the provider entirely when false. */
  when?: (context: FileMenuContext) => boolean
  build: (context: FileMenuContext) => FileMenuSection[]
}

/** System-level configuration applied on top of registered providers. */
export interface FileMenuConfig {
  /** Provider ids to drop entirely. */
  hiddenProviders?: string[]
  /** Action/submenu ids to drop. */
  hiddenItems?: string[]
  /** Per-provider order overrides. */
  providerOrder?: Record<string, number>
}
