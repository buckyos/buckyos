/** Shared display helpers for file entries (icons + formatting). */

import {
  Archive,
  BookOpen,
  Code,
  FileArchive,
  FileAudio,
  FileCode,
  FileImage,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideo,
  File as FileIconGeneric,
  FolderClosed,
  Image as ImageIcon,
  Package,
  Presentation,
  ScrollText,
  Shapes,
  type LucideIcon,
} from 'lucide-react'
import { extensionOf, mediaTypeFromExtension, TEXT_CODE_EXTENSIONS } from 'buckyos/content'
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

/**
 * Finer-grained presentation type than `FileKind` (which stays the search
 * filter's classification): picks the icon and decides whether a thumbnail
 * is worth asking for.
 */
export type FileVisual =
  | 'folder'
  | 'image'
  | 'video'
  | 'audio'
  | 'pdf'
  | 'doc'
  | 'sheet'
  | 'slides'
  | 'text'
  | 'markdown'
  | 'code'
  | 'archive'
  | 'ebook'
  | 'drawing'
  | 'installer'
  | 'file'

const VISUAL_BY_EXT: Record<string, FileVisual> = {}
const table: [FileVisual, string[]][] = [
  ['pdf', ['pdf']],
  ['doc', ['doc', 'docx', 'docm', 'dot', 'dotx', 'odt', 'ott', 'rtf', 'pages', 'wps', 'wpd']],
  ['sheet', ['xls', 'xlsx', 'xlsm', 'xlsb', 'xlt', 'xltx', 'ods', 'ots', 'csv', 'tsv', 'numbers', 'et']],
  ['slides', ['ppt', 'pptx', 'pptm', 'pps', 'ppsx', 'pot', 'potx', 'odp', 'otp', 'key', 'dps']],
  ['markdown', ['md', 'markdown', 'mdx']],
  ['text', ['txt', 'text', 'log', 'ini', 'conf', 'cfg', 'env', 'rst', 'adoc', 'nfo']],
  ['archive', ['zip', 'tar', 'gz', 'tgz', 'bz2', 'tbz2', 'xz', 'txz', 'zst', '7z', 'rar', 'lz', 'lzma', 'cab', 'iso']],
  ['ebook', ['epub', 'mobi', 'azw', 'azw3', 'fb2', 'djvu']],
  ['drawing', ['vsd', 'vsdx', 'vsdm', 'drawio', 'dio', 'dwg', 'dxf', 'sketch', 'fig', 'xmind']],
  ['installer', ['exe', 'msi', 'msix', 'appx', 'dmg', 'pkg', 'mpkg', 'app', 'apk', 'aab', 'xapk', 'ipa', 'deb', 'rpm', 'appimage', 'snap', 'flatpak']],
]
for (const ext of TEXT_CODE_EXTENSIONS) VISUAL_BY_EXT[ext] = 'code'
for (const [visual, exts] of table) for (const ext of exts) VISUAL_BY_EXT[ext] = visual

export function fileVisual(entry: { kind: FileEntry['kind']; name: string; mime?: string }): FileVisual {
  if (entry.kind === 'folder') return 'folder'
  const ext = extensionOf(entry.name)
  const byExt = VISUAL_BY_EXT[ext]
  if (byExt) return byExt
  const mime = (entry.mime || mediaTypeFromExtension(ext) || '').toLowerCase()
  if (mime.startsWith('image/')) return 'image'
  if (mime.startsWith('video/')) return 'video'
  if (mime.startsWith('audio/')) return 'audio'
  if (mime === 'application/pdf') return 'pdf'
  if (mime.startsWith('text/')) return 'text'
  switch (entry.kind) {
    case 'image':
    case 'video':
    case 'audio':
    case 'archive':
    case 'code':
      return entry.kind
    case 'document':
      return 'text'
    default:
      return 'file'
  }
}

/** Icon + theme color per visual type; shapes differ, colors only help. */
const VISUAL_ICON: Record<FileVisual, [LucideIcon, string]> = {
  folder: [FolderClosed, 'var(--cp-accent)'],
  image: [FileImage, 'var(--cp-success)'],
  video: [FileVideo, 'var(--cp-danger)'],
  audio: [FileAudio, 'var(--cp-warning)'],
  pdf: [FileType, 'var(--cp-danger)'],
  doc: [FileText, 'var(--cp-accent)'],
  sheet: [FileSpreadsheet, 'var(--cp-success)'],
  slides: [Presentation, 'var(--cp-warning)'],
  text: [ScrollText, 'var(--cp-muted)'],
  markdown: [FileText, 'var(--cp-text)'],
  code: [FileCode, 'var(--cp-accent)'],
  archive: [FileArchive, 'var(--cp-warning)'],
  ebook: [BookOpen, 'var(--cp-ink-accent)'],
  drawing: [Shapes, 'var(--cp-accent)'],
  installer: [Package, 'var(--cp-text)'],
  file: [FileIconGeneric, 'var(--cp-muted)'],
}

/** The icon for an entry the caller knows the name of (use `kindIcon` when only the kind is known). */
export function fileIcon(entry: { kind: FileEntry['kind']; name: string; mime?: string }, size = 18) {
  const visual = fileVisual(entry)
  const [Icon, color] = VISUAL_ICON[visual]
  return <Icon size={size} style={{ color }} data-file-visual={visual} aria-hidden />
}

/** Types whose thumbnail the Preview Pipeline may give (documents only from an embedded picture or a cached PDF). */
const THUMB_DOCUMENTS = new Set(['docx', 'xlsx', 'pptx', 'odt', 'ods', 'odp', 'pages', 'numbers', 'key', 'epub'])

export function wantsThumbnail(entry: { kind: FileEntry['kind']; name: string; mime?: string }): boolean {
  const visual = fileVisual(entry)
  if (visual === 'image' || visual === 'video' || visual === 'pdf') return true
  return THUMB_DOCUMENTS.has(extensionOf(entry.name))
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
