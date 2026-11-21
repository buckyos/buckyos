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
import { dictionaries } from '../../i18n/dictionaries'
import { matchSupportedLocale } from '../../models/ui'
import type { FileEntry } from './types'

const TYPE_TONE = {
  blue: 'var(--files-type-blue, var(--cp-app-blue))',
  green: 'var(--files-type-green, var(--cp-app-green))',
  amber: 'var(--files-type-amber, var(--cp-app-amber))',
  red: 'var(--files-type-red, var(--cp-app-red))',
  violet: 'var(--files-type-violet, var(--cp-app-violet))',
  slate: 'var(--files-type-slate, var(--cp-app-slate))',
}

export function kindIcon(kind: FileEntry['kind'], size = 18) {
  switch (kind) {
    case 'folder':
      return <FolderClosed size={size} style={{ color: TYPE_TONE.blue }} />
    case 'image':
      return <ImageIcon size={size} style={{ color: TYPE_TONE.green }} />
    case 'video':
      return <FileVideo size={size} style={{ color: TYPE_TONE.violet }} />
    case 'audio':
      return <FileAudio size={size} style={{ color: TYPE_TONE.violet }} />
    case 'archive':
      return <Archive size={size} style={{ color: TYPE_TONE.amber }} />
    case 'code':
      return <Code size={size} style={{ color: TYPE_TONE.slate }} />
    default:
      return <FileText size={size} style={{ color: TYPE_TONE.slate }} />
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

/** Icon + type colour per visual type; shapes differ, colours only help. */
const VISUAL_ICON: Record<FileVisual, [LucideIcon, string]> = {
  folder: [FolderClosed, TYPE_TONE.blue],
  image: [FileImage, TYPE_TONE.green],
  video: [FileVideo, TYPE_TONE.violet],
  audio: [FileAudio, TYPE_TONE.violet],
  pdf: [FileType, TYPE_TONE.red],
  doc: [FileText, TYPE_TONE.blue],
  sheet: [FileSpreadsheet, TYPE_TONE.green],
  slides: [Presentation, TYPE_TONE.amber],
  text: [ScrollText, TYPE_TONE.slate],
  markdown: [FileText, TYPE_TONE.slate],
  code: [FileCode, TYPE_TONE.slate],
  archive: [FileArchive, TYPE_TONE.amber],
  ebook: [BookOpen, TYPE_TONE.violet],
  drawing: [Shapes, TYPE_TONE.violet],
  installer: [Package, TYPE_TONE.slate],
  file: [FileIconGeneric, TYPE_TONE.slate],
}

/** The icon for an entry the caller knows the name of (use `kindIcon` when only the kind is known). */
export function fileIcon(entry: { kind: FileEntry['kind']; name: string; mime?: string }, size = 18) {
  const visual = fileVisual(entry)
  const [Icon, color] = VISUAL_ICON[visual]
  return <Icon size={size} style={{ color }} data-file-visual={visual} aria-hidden />
}

/** Types whose thumbnail the Preview Pipeline may give (documents only from an embedded picture or a cached PDF). */
const THUMB_DOCUMENTS = new Set(['docx', 'xlsx', 'pptx', 'odt', 'ods', 'odp', 'pages', 'numbers', 'key', 'epub', 'drawio', 'dio'])

export function wantsThumbnail(entry: { kind: FileEntry['kind']; name: string; mime?: string }): boolean {
  const visual = fileVisual(entry)
  if (visual === 'image' || visual === 'video' || visual === 'pdf') return true
  return THUMB_DOCUMENTS.has(extensionOf(entry.name))
}

function unknownLabel(locale: string): string {
  const table = dictionaries[matchSupportedLocale(locale) ?? 'en']
  return table['filebrowser.meta.unknown'] ?? dictionaries.en['filebrowser.meta.unknown']
}

export function formatBytes(bytes?: number, locale = document.documentElement.lang || 'en') {
  if (bytes === undefined || !Number.isFinite(bytes)) return unknownLabel(locale)
  const decimal = { minimumFractionDigits: 1, maximumFractionDigits: 1 }
  if (bytes < 1024) return `${bytes.toLocaleString(locale)} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toLocaleString(locale, decimal)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toLocaleString(locale, decimal)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toLocaleString(locale, decimal)} GB`
}

export function formatDate(iso: string, locale = document.documentElement.lang || 'en') {
  const date = new Date(iso)
  if (!Number.isFinite(date.getTime())) return unknownLabel(locale)
  return date.toLocaleString(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}
