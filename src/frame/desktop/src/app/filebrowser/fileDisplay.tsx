/** Shared display helpers for file entries (icons + formatting). */

import {
  Archive,
  Code,
  FileAudio,
  FileText,
  FileVideo,
  FolderClosed,
  Image as ImageIcon,
} from 'lucide-react'
import type { FileEntry } from './types'

export function kindIcon(kind: FileEntry['kind'], size = 18) {
  switch (kind) {
    case 'folder':
      return <FolderClosed size={size} className="text-[color:var(--cp-accent)]" />
    case 'image':
      return <ImageIcon size={size} className="text-[color:var(--cp-success)]" />
    case 'video':
      return <FileVideo size={size} className="text-[color:var(--cp-warning)]" />
    case 'audio':
      return <FileAudio size={size} className="text-[color:var(--cp-warning)]" />
    case 'archive':
      return <Archive size={size} className="text-[color:var(--cp-muted)]" />
    case 'code':
      return <Code size={size} className="text-[color:var(--cp-accent)]" />
    default:
      return <FileText size={size} className="text-[color:var(--cp-muted)]" />
  }
}

export function formatBytes(bytes?: number, locale = document.documentElement.lang || 'en') {
  if (bytes === undefined || !Number.isFinite(bytes)) return locale.startsWith('zh') ? '未知' : 'Unknown'
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`
}

export function formatDate(iso: string, locale = document.documentElement.lang || 'en') {
  const date = new Date(iso)
  if (!Number.isFinite(date.getTime())) return locale.startsWith('zh') ? '未知' : 'Unknown'
  return date.toLocaleString(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}
