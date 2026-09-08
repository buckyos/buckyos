import type { TaskMgrTask } from './task_mgr'
import type { WireRef } from './nfsp_client'

export interface CopyInput {
  sources: { source_ref: WireRef; source_path: string; name: string }[]
  destination_ref: WireRef
  conflict: 'ask' | 'keep-both' | 'skip' | 'cancel'
  retry_of: string | null
}
export interface CopySummary {
  success: number
  failed: number
  skipped: number
  cancelled: number
  pending: number
  bytes: number
}
export interface CopyItem {
  id: number
  source_index: number
  source_path: string
  target_path: string
  kind: string
  size?: number | null
  mtime?: number | null
  status: string
  error?: { code: string; message: string; target_kind?: string; target_size?: number | null; target_mtime?: number | null }
  identity?: { dev: number; ino: number; kind: string }
}
export interface CopyView {
  task: TaskMgrTask
  summary: CopySummary
  items: CopyItem[]
  next: number | null
  conflict: CopyItem | null
}
