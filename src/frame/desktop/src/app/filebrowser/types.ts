/**
 * File browser UI data model.
 *
 * The shapes here are the single source of truth for the prototype and drive
 * the mock data in `./mock/data.ts`.  Everything is derived from the PRD in
 * `product/bucky_file/filebrowser_PRD.md`.
 */

export type FileKind =
  | 'folder'
  | 'image'
  | 'document'
  | 'video'
  | 'audio'
  | 'archive'
  | 'code'
  | 'other'

/** A single entry shown in the main content area (folder or file). */
export interface FileEntry {
  id: string
  name: string
  kind: FileKind
  /** Canonical DFS path. */
  path: string
  /** Optional device/mount anchor for the device view. */
  devicePath?: string
  /** Public URL for files located under a `public` folder. */
  publicUrl?: string
  sizeBytes?: number
  /** ISO string — when the entry was last modified. */
  modifiedAt: string
  /** Entries inside a folder (only populated for expanded tree nodes). */
  children?: FileEntry[]
  /** Whether this entry is inside a folder that triggers AI/KB pipelines. */
  triggersActive?: boolean
  /** Tags derived from Meta / AI. */
  tags?: string[]
  /** Topic ids this entry belongs to. */
  topicIds?: string[]
  /** AI semantic description. */
  summary?: string
  /** Camera / EXIF info for images. */
  exif?: {
    camera?: string
    takenAt?: string
    location?: string
    lens?: string
  }
  /** Source of the file (upload, IM import, shared folder, etc.). */
  source?: {
    type: 'local' | 'telegram' | 'shared' | 'friend-upload' | 'system'
    label: string
  }
  /** Story entries — contextual memories attached to the file. */
  story?: StoryEntry[]
}

export interface StoryEntry {
  id: string
  kind: 'chat' | 'share' | 'session' | 'note'
  title: string
  excerpt: string
  /** ISO date. */
  occurredAt: string
  source?: string
}

/** Left sidebar DFS tree node. */
export interface DfsNode {
  id: string
  name: string
  path: string
  icon?: string
  kind: 'home' | 'public' | 'shared' | 'privacy' | 'generic'
  children?: DfsNode[]
}

/** A device exposed to advanced users in the sidebar. */
export interface DeviceNode {
  id: string
  name: string
  host: string
  status: 'online' | 'offline' | 'syncing'
  roots: { path: string; label: string }[]
}

/** AI Topic grouping. */
export interface Topic {
  id: string
  title: string
  description: string
  /** AI provided reason for grouping. */
  reason: string
  coverageCount: number
  updatedAt: string
  /** Second-level groups inside the topic (source / location / kind / people). */
  groups: TopicGroup[]
}

export interface TopicGroup {
  id: string
  label: string
  axis: 'source' | 'location' | 'kind' | 'people' | 'time'
  fileIds: string[]
}

export interface TriggerRule {
  id: string
  name: string
  event: 'on_new_file_upload' | 'on_new_topic_created' | 'on_file_tagged'
  appliesTo: string[]
  pipeline: string
  enabled: boolean
  reason: string
}

export interface SearchHit {
  entryId: string
  /** Why this entry surfaced. */
  reason: 'filename' | 'folder' | 'fulltext' | 'ai_semantic' | 'ai_topic'
  snippet: string
  score: number
}

export type ViewMode = 'list' | 'icon'

export interface BrowserTab {
  id: string
  title: string
  path: string
}

export interface FileBrowserSnapshot {
  dfsRoots: DfsNode[]
  devices: DeviceNode[]
  topics: Topic[]
  triggers: TriggerRule[]
  entriesByPath: Record<string, FileEntry[]>
  entriesById: Record<string, FileEntry>
}
