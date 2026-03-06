import type { ChangeEventHandler, DragEvent, MouseEvent as ReactMouseEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  ArrowUp,
  Copy,
  Download,
  ExternalLink,
  File,
  FileArchive,
  FileCode,
  FileImage,
  FileMusic,
  FilePlus2,
  FileSpreadsheet,
  FileText,
  FileType,
  FileVideoCamera,
  Folder,
  FolderPlus,
  Link,
  List,
  LayoutGrid,
  Clock3,
  Filter,
  Move,
  Pause,
  PencilLine,
  Play,
  Star,
  Undo2,
  RotateCw,
  Save,
  Search,
  Share2,
  Trash2,
  Upload,
  type LucideIcon,
  X,
} from 'lucide-react'
import mammoth from 'mammoth/mammoth.browser'
import useSWR from 'swr'

import { ensureSessionToken } from '@/auth/authManager'
import { getSessionTokenFromCookies, getStoredSessionToken } from '@/auth/session'
import ActionDialog from '@/ui/components/file_manager/ActionDialog'
import FilePreviewPanel from '@/ui/components/file_manager/FilePreviewPanel'
import { downloadImageWithProgress } from '@/ui/components/file_manager/imageDownload'
import { renderMarkdownHtml } from '@/ui/components/file_manager/markdownPreview'
import ProgressRing from '@/ui/components/file_manager/ProgressRing'
import ImageViewerModal from '@/ui/components/file_manager/ImageViewerModal'
import { getFileExtension, getFilePreviewKind, getTextPreviewMode, isDocFileName, type FilePreviewKind } from '@/ui/components/file_manager/filePreview'

type FileEntry = {
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: number
}

type DirectoryResponse = {
  path: string
  is_dir: boolean
  items: FileEntry[]
}

type FileResponse = {
  path: string
  is_dir: boolean
  size: number
  modified: number
  content?: string | null
}

type RecentFileEntry = {
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: number
  last_accessed_at: number
  access_count: number
}

type RecycleBinEntry = {
  item_id: string
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: number
  original_path: string
  deleted_at: number
}

type SearchResponse = {
  query: string
  path: string
  kind: 'all' | 'file' | 'dir'
  limit: number
  truncated: boolean
  items: FileEntry[]
}

type FavoriteListResponse = {
  items: FileEntry[]
}

type RecentListResponse = {
  items: RecentFileEntry[]
}

type RecycleBinListResponse = {
  items: RecycleBinEntry[]
}

type ShareItem = {
  id: string
  owner: string
  path: string
  created_at: number
  expires_at?: number | null
  password_required: boolean
}

type PublicShareResponse = {
  share: ShareItem
  is_dir: boolean
  path?: string
  parent_path?: string
  items?: FileEntry[]
  size?: number
  modified?: number
  content?: string | null
  mime?: string
}

type UploadSessionRecord = {
  id: string
  owner: string
  path: string
  size: number
  chunk_size: number
  uploaded_size: number
  override_existing: boolean
  created_at: number
  updated_at: number
}

type UploadProgressItem = {
  key: string
  name: string
  uploaded: number
  total: number
  status: 'uploading' | 'paused' | 'completed' | 'error' | 'cancelled'
  error?: string
}

type ConfirmDialogState = {
  title: string
  description: ReactNode
  confirmLabel: string
  confirmTone?: 'primary' | 'danger'
  onConfirm: () => Promise<void> | void
}

type MoveCopyDialogState = {
  mode: 'single' | 'batch'
  action: 'move' | 'copy'
  sourceEntry: FileEntry | null
  destination: string
  overrideExisting: boolean
  error: string
}

type ShareDialogState = {
  entry: FileEntry
  expiresInSeconds: string
  password: string
  error: string
}

const DEFAULT_UPLOAD_CHUNK_SIZE = 2 * 1024 * 1024
const UPLOAD_CONCURRENCY = 2
const UPLOAD_MAX_RETRY = 3
const UPLOAD_RETRY_BASE_DELAY_MS = 600
const PUBLIC_TEXT_PREVIEW_LIMIT = 200_000
const ROW_ACTION_MENU_WIDTH = 176
const ROW_ACTION_MENU_ESTIMATED_HEIGHT = 292
const ROW_ACTION_MENU_GAP = 6
const ROW_ACTION_MENU_VIEWPORT_PADDING = 8
const TOUCH_LONG_PRESS_DELAY_MS = 450
const TOUCH_LONG_PRESS_MOVE_TOLERANCE = 10
const LONG_PRESS_CLICK_SUPPRESS_MS = 700

const getPublicShareIdFromPath = (path: string) => {
  const normalizedPath = path.endsWith('/') && path !== '/' ? path.slice(0, -1) : path
  const match = normalizedPath.match(/^\/share\/([^/]+)$/)
  return match?.[1] ? decodeURIComponent(match[1]) : ''
}

const getSearchParam = (key: string) => new URLSearchParams(window.location.search).get(key) ?? ''

const withAuthHeaders = (authToken: string, extraHeaders?: Record<string, string>) => {
  const headers: Record<string, string> = {
    ...(extraHeaders ?? {}),
  }
  if (authToken.trim()) {
    headers['X-Auth'] = authToken.trim()
  }
  return headers
}

type ApiRequestError = Error & {
  status: number
}

const fetchAuthedJson = async <T,>(url: string, authToken: string): Promise<T> => {
  const response = await fetch(url, {
    headers: withAuthHeaders(authToken),
  })

  if (!response.ok) {
    const payload = (await response.json().catch(() => ({}))) as { error?: string }
    const error = new Error(payload.error ?? `Request failed (${response.status})`) as ApiRequestError
    error.status = response.status
    throw error
  }

  return (await response.json()) as T
}

const buildPublicSharePath = (shareId: string) => `/share/${encodeURIComponent(shareId)}`

const buildPublicDownloadPath = (shareId: string, password?: string) => {
  const query = password?.trim()
    ? `?password=${encodeURIComponent(password.trim())}&download=1`
    : '?download=1'
  return `/api/public/dl/${encodeURIComponent(shareId)}${query}`
}

const buildPublicDownloadPathForTarget = (shareId: string, targetPath: string, password?: string) => {
  const query = new URLSearchParams()
  if (password?.trim()) {
    query.set('password', password.trim())
  }
  query.set('download', '1')
  if (targetPath && targetPath !== '/') {
    query.set('path', targetPath)
  }
  const suffix = query.toString()
  return `/api/public/dl/${encodeURIComponent(shareId)}${suffix ? `?${suffix}` : ''}`
}

const buildPublicInlinePathForTarget = (shareId: string, targetPath: string, password?: string) => {
  const query = new URLSearchParams()
  if (password?.trim()) {
    query.set('password', password.trim())
  }
  query.set('inline', '1')
  if (targetPath && targetPath !== '/') {
    query.set('path', targetPath)
  }
  const suffix = query.toString()
  return `/api/public/dl/${encodeURIComponent(shareId)}${suffix ? `?${suffix}` : ''}`
}

const buildFileDetailPath = (path: string) =>
  `/files/detail?path=${encodeURIComponent(normalizeUrlPath(path))}`

const buildPublicShareApiPath = (shareId: string, path: string, password?: string) => {
  const query = new URLSearchParams()
  if (path && path !== '/') {
    query.set('path', path)
  }
  if (password?.trim()) {
    query.set('password', password.trim())
  }
  const suffix = query.toString()
  return `/api/public/share/${encodeURIComponent(shareId)}${suffix ? `?${suffix}` : ''}`
}

const getUploadResumeKey = (targetPath: string, size: number, lastModified: number) =>
  `bucky-file-upload-session:${targetPath}:${size}:${lastModified}`

const revokeBlobUrl = (url: string) => {
  if (!url.startsWith('blob:')) {
    return
  }
  URL.revokeObjectURL(url)
}

type MainTab = 'files' | 'shares' | 'editor'
type FilesViewMode = 'icon' | 'list'
type FilesScope = 'browse' | 'recent' | 'starred' | 'trash'

const getMainTabPath = (tab: MainTab) => {
  if (tab === 'shares') {
    return '/files/shares'
  }
  if (tab === 'editor') {
    return '/files/editor'
  }
  return '/files'
}

const getMainTabFromPathname = (pathname: string): MainTab => {
  if (pathname.startsWith('/files/shares')) {
    return 'shares'
  }
  if (pathname.startsWith('/files/editor')) {
    return 'editor'
  }
  return 'files'
}

const encodePath = (path: string) =>
  path
    .split('/')
    .map((segment, index) => (index === 0 ? '' : encodeURIComponent(segment)))
    .join('/')

const normalizeDirPath = (path: string) => {
  if (!path || path === '/') {
    return '/'
  }
  return path.endsWith('/') ? path : `${path}/`
}

const joinPath = (base: string, name: string) => `${normalizeDirPath(base)}${name}`

const fileNameFromPath = (path: string) => {
  const parts = path.split('/').filter(Boolean)
  return parts[parts.length - 1] ?? ''
}

const renamePath = (path: string, newName: string) => {
  const base = parentPath(path)
  return base === '/' ? `/${newName}` : `${base}/${newName}`
}

const parentPath = (path: string) => {
  if (!path || path === '/') {
    return '/'
  }
  const trimmed = path.endsWith('/') ? path.slice(0, -1) : path
  const index = trimmed.lastIndexOf('/')
  if (index <= 0) {
    return '/'
  }
  return trimmed.slice(0, index)
}

const normalizeUrlPath = (path: string) => {
  if (!path || path.trim() === '') {
    return '/'
  }

  let normalized = path.trim()
  if (!normalized.startsWith('/')) {
    normalized = `/${normalized}`
  }
  normalized = normalized.replace(/\/{2,}/g, '/')
  if (normalized.length > 1 && normalized.endsWith('/')) {
    normalized = normalized.slice(0, -1)
  }
  return normalized || '/'
}

const ellipsizeMiddle = (value: string, maxChars: number) => {
  if (value.length <= maxChars) {
    return value
  }

  const safeMax = Math.max(8, maxChars)
  const keepTotal = safeMax - 3
  const headLen = Math.ceil(keepTotal / 2)
  const tailLen = Math.floor(keepTotal / 2)

  return `${value.slice(0, headLen)}...${value.slice(-tailLen)}`
}

const formatDisplayFileName = (name: string, maxChars = 44) => {
  if (name.length <= maxChars) {
    return name
  }

  const dotIndex = name.lastIndexOf('.')
  const hasExtension = dotIndex > 0 && dotIndex < name.length - 1
  if (!hasExtension) {
    return ellipsizeMiddle(name, maxChars)
  }

  const extension = name.slice(dotIndex)
  const stem = name.slice(0, dotIndex)
  const keepTotal = Math.max(8, maxChars) - 3
  const minHeadLen = 6
  const preferredTailLen = Math.max(10, Math.floor(keepTotal * 0.45))
  const tailLen = Math.min(keepTotal - minHeadLen, Math.max(extension.length + 2, preferredTailLen))

  if (tailLen <= extension.length) {
    return ellipsizeMiddle(name, maxChars)
  }

  const stemTailLen = tailLen - extension.length
  const suffix = `${stem.slice(-stemTailLen)}${extension}`
  const headLen = keepTotal - suffix.length
  if (headLen < minHeadLen) {
    return ellipsizeMiddle(name, maxChars)
  }

  return `${stem.slice(0, headLen)}...${suffix}`
}

const splitNameAndExtension = (name: string) => {
  const lastDotIndex = name.lastIndexOf('.')
  if (lastDotIndex <= 0 || lastDotIndex >= name.length - 1) {
    return {
      baseName: name,
      extension: '',
    }
  }

  return {
    baseName: name.slice(0, lastDotIndex),
    extension: name.slice(lastDotIndex + 1),
  }
}

const buildNameFromParts = (baseName: string, extension: string) => {
  const cleanedBaseName = baseName.trim()
  const cleanedExtension = extension.trim().replace(/^\.+/, '')
  if (!cleanedExtension) {
    return cleanedBaseName
  }
  return `${cleanedBaseName}.${cleanedExtension}`
}

type FileNameTooltipProps = {
  name: string
  maxChars?: number
  maxWidthClass?: string
}

type HoverTooltipProps = {
  label: string
  children: ReactNode
}

const HoverTooltip = ({ label, children }: HoverTooltipProps) => (
  <span className="group/hover-tooltip relative inline-flex">
    {children}
    <span className="pointer-events-none invisible absolute left-1/2 top-full z-[80] mt-2 -translate-x-1/2 whitespace-nowrap rounded-md bg-slate-950 px-2 py-1 text-[11px] font-semibold text-white opacity-0 shadow-lg transition-opacity duration-150 group-hover/hover-tooltip:visible group-hover/hover-tooltip:opacity-100 group-focus-within/hover-tooltip:visible group-focus-within/hover-tooltip:opacity-100">
      {label}
    </span>
  </span>
)

const FileNameTooltip = ({ name, maxChars = 44, maxWidthClass = 'max-w-[420px]' }: FileNameTooltipProps) => {
  const displayName = formatDisplayFileName(name, maxChars)
  const hasOverflow = displayName !== name

  return (
    <span className={`group/file-name relative inline-flex min-w-0 ${maxWidthClass} items-center overflow-hidden`}>
      <span className="block w-full truncate">{displayName}</span>
      {hasOverflow ? (
        <span className="pointer-events-none invisible absolute top-full left-0 z-[70] mt-1 w-max max-w-[min(80vw,560px)] break-all rounded-lg border border-slate-200 bg-slate-900/95 px-2.5 py-1.5 text-[11px] font-medium leading-relaxed text-white opacity-0 shadow-lg transition-opacity duration-150 group-hover/file-name:visible group-hover/file-name:opacity-100">
          {name}
        </span>
      ) : null}
    </span>
  )
}

const formatBytes = (value: number) => {
  if (!Number.isFinite(value) || value <= 0) {
    return '-'
  }
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  const unitIndex = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  const scaled = value / 1024 ** unitIndex
  return `${scaled.toFixed(scaled >= 100 || unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
}

const formatTimestamp = (value: number) => {
  if (!Number.isFinite(value) || value <= 0) {
    return '-'
  }
  return new Date(value * 1000).toLocaleString()
}

const ARCHIVE_EXTENSIONS = new Set(['zip', 'tar', 'gz', 'tgz', 'bz2', 'xz', '7z', 'rar'])
const CODE_EXTENSIONS = new Set([
  'js',
  'jsx',
  'ts',
  'tsx',
  'mjs',
  'cjs',
  'rs',
  'py',
  'go',
  'java',
  'kt',
  'swift',
  'c',
  'h',
  'cpp',
  'hpp',
  'sh',
  'bash',
  'zsh',
])
const DOCUMENT_EXTENSIONS = new Set(['doc', 'docx', 'odt', 'rtf'])
const SPREADSHEET_EXTENSIONS = new Set(['xls', 'xlsx', 'csv', 'ods'])
const PRESENTATION_EXTENSIONS = new Set(['ppt', 'pptx', 'odp'])

type EntryIconMeta = {
  iconName: IconName
  iconClassName?: string
}

const getEntryIconMeta = (entry: { name: string; is_dir: boolean }): EntryIconMeta => {
  if (entry.is_dir) {
    return { iconName: 'folder', iconClassName: 'text-amber-500' }
  }

  const extension = getFileExtension(entry.name)
  const previewKind = getFilePreviewKind(entry)

  if (previewKind === 'image') {
    return { iconName: 'file-image', iconClassName: 'text-sky-600' }
  }
  if (extension === 'pdf') {
    return { iconName: 'file-text', iconClassName: 'text-rose-600' }
  }
  if (previewKind === 'audio') {
    return { iconName: 'file-music', iconClassName: 'text-violet-600' }
  }
  if (previewKind === 'video') {
    return { iconName: 'file-video', iconClassName: 'text-fuchsia-600' }
  }
  if (DOCUMENT_EXTENSIONS.has(extension) || PRESENTATION_EXTENSIONS.has(extension)) {
    return { iconName: 'file-doc', iconClassName: 'text-blue-600' }
  }
  if (SPREADSHEET_EXTENSIONS.has(extension)) {
    return { iconName: 'file-sheet', iconClassName: 'text-emerald-600' }
  }
  if (ARCHIVE_EXTENSIONS.has(extension)) {
    return { iconName: 'file-archive', iconClassName: 'text-orange-600' }
  }
  if (previewKind === 'text' || CODE_EXTENSIONS.has(extension)) {
    return { iconName: 'file-code', iconClassName: 'text-cyan-700' }
  }

  return { iconName: 'file', iconClassName: 'text-slate-500' }
}

type IconName =
  | 'open'
  | 'up'
  | 'search'
  | 'clear'
  | 'upload'
  | 'download'
  | 'new-folder'
  | 'new-file'
  | 'move'
  | 'copy'
  | 'delete'
  | 'pause'
  | 'resume'
  | 'retry'
  | 'rename'
  | 'share'
  | 'link'
  | 'save'
  | 'close'
  | 'folder'
  | 'file'
  | 'file-text'
  | 'file-image'
  | 'file-doc'
  | 'file-sheet'
  | 'file-code'
  | 'file-archive'
  | 'file-music'
  | 'file-video'
  | 'view-list'
  | 'view-icon'
  | 'scope-recent'
  | 'scope-starred'
  | 'scope-trash'
  | 'filter'
  | 'restore'
  | 'star'

const Icon = ({ name, className = '' }: { name: IconName; className?: string }) => {
  const icons: Record<IconName, LucideIcon> = {
    open: ExternalLink,
    up: ArrowUp,
    search: Search,
    clear: X,
    upload: Upload,
    download: Download,
    'new-folder': FolderPlus,
    'new-file': FilePlus2,
    move: Move,
    copy: Copy,
    delete: Trash2,
    pause: Pause,
    resume: Play,
    retry: RotateCw,
    rename: PencilLine,
    share: Share2,
    link: Link,
    save: Save,
    close: X,
    folder: Folder,
    file: File,
    'file-text': FileText,
    'file-image': FileImage,
    'file-doc': FileType,
    'file-sheet': FileSpreadsheet,
    'file-code': FileCode,
    'file-archive': FileArchive,
    'file-music': FileMusic,
    'file-video': FileVideoCamera,
    'view-list': List,
    'view-icon': LayoutGrid,
    'scope-recent': Clock3,
    'scope-starred': Star,
    'scope-trash': Trash2,
    filter: Filter,
    restore: Undo2,
    star: Star,
  }

  const Lucide = icons[name]
  return <Lucide className={`size-4 shrink-0 ${className}`} aria-hidden />
}

type FileManagerPageProps = {
  embedded?: boolean
}

const FileManagerPage = ({ embedded = false }: FileManagerPageProps) => {
  const [token, setToken] = useState(() => getStoredSessionToken() || getSessionTokenFromCookies() || '')
  const [locationPathname, setLocationPathname] = useState(() => (embedded ? '/desktop/files' : window.location.pathname))
  const publicShareId = useMemo(() => getPublicShareIdFromPath(locationPathname), [locationPathname])
  const [mainTab, setMainTab] = useState<MainTab>(() => (embedded ? 'files' : getMainTabFromPathname(window.location.pathname)))
  const [currentPath, setCurrentPath] = useState(() => {
    if (embedded) {
      return '/'
    }
    if (getPublicShareIdFromPath(window.location.pathname)) {
      return '/'
    }
    return normalizeUrlPath(getSearchParam('path') || '/')
  })
  const [currentPathIsDir, setCurrentPathIsDir] = useState(true)
  const [filesViewMode, setFilesViewMode] = useState<FilesViewMode>('icon')
  const [filesScope, setFilesScope] = useState<FilesScope>('browse')
  const [items, setItems] = useState<FileEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [message, setMessage] = useState('')
  const [deleteToast, setDeleteToast] = useState('')
  const [selectedPaths, setSelectedPaths] = useState<string[]>([])
  const [thumbLoadFailed, setThumbLoadFailed] = useState<Record<string, boolean>>({})
  const [editorPath, setEditorPath] = useState('')
  const [editorContent, setEditorContent] = useState('')
  const [editorDirty, setEditorDirty] = useState(false)
  const [editorSaving, setEditorSaving] = useState(false)
  const [renameTarget, setRenameTarget] = useState<FileEntry | null>(null)
  const [renameBaseName, setRenameBaseName] = useState('')
  const [renameExtension, setRenameExtension] = useState('')
  const [renameEditingExtension, setRenameEditingExtension] = useState(false)
  const [renameSubmitting, setRenameSubmitting] = useState(false)
  const [renameError, setRenameError] = useState('')
  const [confirmDialog, setConfirmDialog] = useState<ConfirmDialogState | null>(null)
  const [confirmDialogBusy, setConfirmDialogBusy] = useState(false)
  const [moveCopyDialog, setMoveCopyDialog] = useState<MoveCopyDialogState | null>(null)
  const [moveCopyDialogBusy, setMoveCopyDialogBusy] = useState(false)
  const [folderDialogOpen, setFolderDialogOpen] = useState(false)
  const [folderNameInput, setFolderNameInput] = useState('')
  const [folderDialogError, setFolderDialogError] = useState('')
  const [folderDialogBusy, setFolderDialogBusy] = useState(false)
  const [shareDialog, setShareDialog] = useState<ShareDialogState | null>(null)
  const [shareDialogBusy, setShareDialogBusy] = useState(false)
  const [searchKeyword, setSearchKeyword] = useState('')
  const [showAdvancedFilters, setShowAdvancedFilters] = useState(false)
  const [filterKind, setFilterKind] = useState<'all' | 'file' | 'dir'>('all')
  const [filterExtInput, setFilterExtInput] = useState('')
  const [filterDatePreset, setFilterDatePreset] = useState<'all' | '7d' | '30d' | 'custom'>('all')
  const [filterDateFrom, setFilterDateFrom] = useState('')
  const [filterDateTo, setFilterDateTo] = useState('')
  const [filterSizeMin, setFilterSizeMin] = useState('')
  const [filterSizeMax, setFilterSizeMax] = useState('')
  const [filterSortBy, setFilterSortBy] = useState<'name' | 'modified' | 'size'>('name')
  const [filterSortOrder, setFilterSortOrder] = useState<'asc' | 'desc'>('asc')
  const [searchLoading, setSearchLoading] = useState(false)
  const [searchResults, setSearchResults] = useState<FileEntry[]>([])
  const [searchTruncated, setSearchTruncated] = useState(false)
  const [searchActive, setSearchActive] = useState(false)
  const [publicSharePassword, setPublicSharePassword] = useState(() => (embedded ? '' : getSearchParam('password')))
  const [publicShareLoading, setPublicShareLoading] = useState(false)
  const [publicShareError, setPublicShareError] = useState('')
  const [publicShareData, setPublicShareData] = useState<PublicShareResponse | null>(null)
  const [publicSharePath, setPublicSharePath] = useState(() => normalizeUrlPath(embedded ? '/' : getSearchParam('path') || '/'))
  const [publicPreviewExpanded, setPublicPreviewExpanded] = useState(false)
  const [uploadProgress, setUploadProgress] = useState<UploadProgressItem[]>([])
  const [uploadPaused, setUploadPaused] = useState(false)
  const [uploadPanelOpen, setUploadPanelOpen] = useState(false)
  const [dropzoneActive, setDropzoneActive] = useState(false)
  const [previewEntry, setPreviewEntry] = useState<FileEntry | null>(null)
  const [previewKind, setPreviewKind] = useState<FilePreviewKind>('unknown')
  const [previewTextContent, setPreviewTextContent] = useState('')
  const [previewDocxHtml, setPreviewDocxHtml] = useState('')
  const [previewDocPdfSrc, setPreviewDocPdfSrc] = useState('')
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewLoadingLabel, setPreviewLoadingLabel] = useState('Loading preview...')
  const [previewLoadingElapsedSeconds, setPreviewLoadingElapsedSeconds] = useState(0)
  const [previewError, setPreviewError] = useState('')
  const [imageViewerOpen, setImageViewerOpen] = useState(false)
  const [imageViewerSrc, setImageViewerSrc] = useState('')
  const [imageViewerTitle, setImageViewerTitle] = useState('')
  const [imageViewerScale, setImageViewerScale] = useState(1)
  const [previewImageLoading, setPreviewImageLoading] = useState(false)
  const [previewImageSrc, setPreviewImageSrc] = useState('')
  const [previewImageLoadedBytes, setPreviewImageLoadedBytes] = useState(0)
  const [previewImageTotalBytes, setPreviewImageTotalBytes] = useState<number | null>(null)
  const [previewImageProgressPercent, setPreviewImageProgressPercent] = useState<number | null>(null)
  const [publicImageLoading, setPublicImageLoading] = useState(false)
  const [publicImageDisplaySrc, setPublicImageDisplaySrc] = useState('')
  const [publicImageLoadedBytes, setPublicImageLoadedBytes] = useState(0)
  const [publicImageTotalBytes, setPublicImageTotalBytes] = useState<number | null>(null)
  const [publicImageProgressPercent, setPublicImageProgressPercent] = useState<number | null>(null)
  const [viewerImageLoading, setViewerImageLoading] = useState(false)
  const [openActionPath, setOpenActionPath] = useState('')
  const [actionMenuPosition, setActionMenuPosition] = useState<{ top: number; left: number } | null>(null)

  const effectiveToken = token || getStoredSessionToken() || getSessionTokenFromCookies() || ''
  const uploadPausedRef = useRef(false)
  const dropDragDepthRef = useRef(0)
  const uploadFilesRef = useRef(new Map<string, { file: File; targetPath: string; resumeKey: string }>())
  const uploadSessionRef = useRef(new Map<string, string>())
  const uploadCancelledRef = useRef(new Set<string>())
  const rowActionMenuRef = useRef<HTMLDivElement | null>(null)
  const renameExtensionInputRef = useRef<HTMLInputElement | null>(null)
  const deleteToastTimerRef = useRef<number | null>(null)
  const touchLongPressTimerRef = useRef<number | null>(null)
  const touchLongPressStateRef = useRef<{
    pointerId: number
    entryPath: string
    startX: number
    startY: number
  } | null>(null)
  const suppressClickUntilRef = useRef(0)

  const enableListRequests = Boolean(effectiveToken && !publicShareId)
  const listToken = effectiveToken.trim()

  const {
    data: sharesPayload,
    error: sharesRequestError,
    isLoading: sharesLoading,
    mutate: mutateShares,
  } = useSWR(
    enableListRequests ? ['/api/share', listToken] : null,
    ([url, authToken]) => fetchAuthedJson<{ items?: ShareItem[] }>(url, authToken),
    {
      revalidateOnFocus: false,
      dedupingInterval: 4000,
    },
  )

  const {
    data: favoritesPayload,
    error: favoritesRequestError,
    mutate: mutateFavorites,
  } = useSWR(
    enableListRequests ? ['/api/favorites?limit=500', listToken] : null,
    ([url, authToken]) => fetchAuthedJson<FavoriteListResponse>(url, authToken),
    {
      revalidateOnFocus: false,
      dedupingInterval: 4000,
    },
  )

  const {
    data: recentPayload,
    error: recentRequestError,
    mutate: mutateRecent,
  } = useSWR(
    enableListRequests ? ['/api/recent?limit=500', listToken] : null,
    ([url, authToken]) => fetchAuthedJson<RecentListResponse>(url, authToken),
    {
      revalidateOnFocus: false,
      dedupingInterval: 4000,
    },
  )

  const {
    data: trashPayload,
    error: trashRequestError,
    mutate: mutateTrash,
  } = useSWR(
    enableListRequests ? ['/api/recycle-bin?limit=500', listToken] : null,
    ([url, authToken]) => fetchAuthedJson<RecycleBinListResponse>(url, authToken),
    {
      revalidateOnFocus: false,
      dedupingInterval: 4000,
    },
  )

  const shares = useMemo(() => (Array.isArray(sharesPayload?.items) ? sharesPayload.items : []), [sharesPayload])
  const favoriteItems = useMemo(
    () => (Array.isArray(favoritesPayload?.items) ? favoritesPayload.items : []),
    [favoritesPayload],
  )
  const recentItems = useMemo(
    () => (Array.isArray(recentPayload?.items) ? recentPayload.items : []),
    [recentPayload],
  )
  const trashItems = useMemo(() => (Array.isArray(trashPayload?.items) ? trashPayload.items : []), [trashPayload])

  const showDeleteToast = useCallback((text: string) => {
    setDeleteToast(text)
  }, [])

  useEffect(() => {
    if (!deleteToast) {
      return
    }

    if (deleteToastTimerRef.current != null) {
      window.clearTimeout(deleteToastTimerRef.current)
    }

    deleteToastTimerRef.current = window.setTimeout(() => {
      setDeleteToast('')
      deleteToastTimerRef.current = null
    }, 2600)

    return () => {
      if (deleteToastTimerRef.current != null) {
        window.clearTimeout(deleteToastTimerRef.current)
        deleteToastTimerRef.current = null
      }
    }
  }, [deleteToast])

  const closeRowActionMenu = useCallback(() => {
    setOpenActionPath('')
    setActionMenuPosition(null)
  }, [])

  const closeRenameModal = useCallback((force = false) => {
    if (renameSubmitting && !force) {
      return
    }
    setRenameTarget(null)
    setRenameBaseName('')
    setRenameExtension('')
    setRenameEditingExtension(false)
    setRenameError('')
  }, [renameSubmitting])

  const closeConfirmDialog = useCallback(() => {
    if (confirmDialogBusy) {
      return
    }
    setConfirmDialog(null)
  }, [confirmDialogBusy])

  const closeMoveCopyDialog = useCallback(() => {
    if (moveCopyDialogBusy) {
      return
    }
    setMoveCopyDialog(null)
  }, [moveCopyDialogBusy])

  const closeFolderDialog = useCallback((force = false) => {
    if (folderDialogBusy && !force) {
      return
    }
    setFolderDialogOpen(false)
    setFolderNameInput('')
    setFolderDialogError('')
  }, [folderDialogBusy])

  const closeShareDialog = useCallback(() => {
    if (shareDialogBusy) {
      return
    }
    setShareDialog(null)
  }, [shareDialogBusy])

  const submitConfirmDialog = useCallback(async () => {
    if (!confirmDialog) {
      return
    }

    setConfirmDialogBusy(true)
    try {
      await confirmDialog.onConfirm()
      setConfirmDialog(null)
    } finally {
      setConfirmDialogBusy(false)
    }
  }, [confirmDialog])

  const onRename = useCallback((entry: FileEntry) => {
    const { baseName, extension } = splitNameAndExtension(entry.name)
    setRenameTarget(entry)
    setRenameBaseName(baseName)
    setRenameExtension(extension)
    setRenameEditingExtension(false)
    setRenameError('')
  }, [])

  const clearSearchState = useCallback(() => {
    setSearchActive(false)
    setSearchResults([])
    setSearchTruncated(false)
  }, [])

  const clearPreviewState = useCallback(() => {
    setPreviewDocPdfSrc((prev) => {
      revokeBlobUrl(prev)
      return ''
    })
    setPreviewImageSrc((prev) => {
      revokeBlobUrl(prev)
      return ''
    })
    setPreviewImageLoadedBytes(0)
    setPreviewImageTotalBytes(null)
    setPreviewImageProgressPercent(null)
    setPreviewEntry(null)
    setPreviewKind('unknown')
    setPreviewTextContent('')
    setPreviewDocxHtml('')
    setPreviewError('')
    setPreviewLoading(false)
    setPreviewLoadingLabel('Loading preview...')
    setPreviewLoadingElapsedSeconds(0)
    setPreviewImageLoading(false)
    setImageViewerOpen(false)
  }, [])

  const openImageViewer = useCallback((src: string, title: string) => {
    if (!src) {
      return
    }
    setImageViewerSrc(src)
    setImageViewerTitle(title)
    setImageViewerScale(1)
    setViewerImageLoading(true)
    setImageViewerOpen(true)
  }, [])

  const closeImageViewer = useCallback(() => {
    setImageViewerOpen(false)
    setViewerImageLoading(false)
  }, [])

  const zoomInImage = useCallback(() => {
    setImageViewerScale((prev) => Math.min(4, Number((prev + 0.25).toFixed(2))))
  }, [])

  const zoomOutImage = useCallback(() => {
    setImageViewerScale((prev) => Math.max(0.5, Number((prev - 0.25).toFixed(2))))
  }, [])

  const resetImageZoom = useCallback(() => {
    setImageViewerScale(1)
  }, [])

  const openDirectory = useCallback(
    (path: string) => {
      clearSearchState()
      clearPreviewState()
      setCurrentPath(normalizeUrlPath(path))
    },
    [clearPreviewState, clearSearchState],
  )

  const openDirectoryInBrowse = useCallback(
    (path: string) => {
      setFilesScope('browse')
      openDirectory(path)
    },
    [openDirectory],
  )

  const syncMainPathToUrl = useCallback(
    (path: string) => {
      if (embedded) {
        return
      }
      if (publicShareId || mainTab !== 'files') {
        return
      }
      const normalized = normalizeUrlPath(path)
      const url = new URL(window.location.href)
      url.pathname = '/files'
      if (normalized === '/') {
        url.searchParams.delete('path')
      } else {
        url.searchParams.set('path', normalized)
      }

      const nextSearch = url.searchParams.toString()
      const next = `${url.pathname}${nextSearch ? `?${nextSearch}` : ''}${url.hash}`
      const current = `${window.location.pathname}${window.location.search}${window.location.hash}`
      if (next !== current) {
        window.history.pushState(null, '', next)
        setLocationPathname(url.pathname)
      }
    },
    [embedded, mainTab, publicShareId],
  )

  const syncPublicSharePathToUrl = useCallback(
    (path: string, passwordInput: string) => {
      if (!publicShareId) {
        return
      }

      const normalized = normalizeUrlPath(path)
      const url = new URL(window.location.href)
      if (normalized === '/') {
        url.searchParams.delete('path')
      } else {
        url.searchParams.set('path', normalized)
      }

      const passwordValue = passwordInput.trim()
      if (passwordValue) {
        url.searchParams.set('password', passwordValue)
      } else {
        url.searchParams.delete('password')
      }

      const nextSearch = url.searchParams.toString()
      const next = `${url.pathname}${nextSearch ? `?${nextSearch}` : ''}${url.hash}`
      const current = `${window.location.pathname}${window.location.search}${window.location.hash}`
      if (next !== current) {
        window.history.pushState(null, '', next)
        setLocationPathname(url.pathname)
      }
    },
    [publicShareId],
  )

  const navigateToMainTab = useCallback(
    (tab: MainTab) => {
      if (publicShareId) {
        return
      }

      if (embedded) {
        setMainTab(tab)
        return
      }

      const url = new URL(window.location.href)
      url.pathname = getMainTabPath(tab)
      if (tab === 'files') {
        const normalized = normalizeUrlPath(currentPath)
        if (normalized === '/') {
          url.searchParams.delete('path')
        } else {
          url.searchParams.set('path', normalized)
        }
      } else {
        url.searchParams.delete('path')
      }

      const nextSearch = url.searchParams.toString()
      const next = `${url.pathname}${nextSearch ? `?${nextSearch}` : ''}${url.hash}`
      const current = `${window.location.pathname}${window.location.search}${window.location.hash}`
      if (next !== current) {
        window.history.pushState(null, '', next)
      }
      setLocationPathname(url.pathname)
      setMainTab(tab)
    },
    [currentPath, embedded, publicShareId],
  )

  const setSessionToken = useCallback((next: string) => {
    setToken(next)
  }, [])

  const handleListRequestError = useCallback((error: unknown, fallbackMessage: string) => {
    if (!error) {
      return
    }
    const requestError = error as ApiRequestError
    if (requestError.status === 401) {
      setSessionToken('')
      setMessage('会话已失效，请在 Control Panel 重新登录。')
      return
    }
    setMessage(requestError.message || fallbackMessage)
  }, [setSessionToken])

  useEffect(() => {
    let cancelled = false

    if (token) {
      return () => {
        cancelled = true
      }
    }

    const storedToken = getStoredSessionToken() || getSessionTokenFromCookies()
    if (storedToken) {
      setSessionToken(storedToken)
      return () => {
        cancelled = true
      }
    }

    if (!embedded && publicShareId) {
      return () => {
        cancelled = true
      }
    }

    void ensureSessionToken().then((nextToken) => {
      if (!cancelled && nextToken) {
        setSessionToken(nextToken)
      }
    })

    return () => {
      cancelled = true
    }
  }, [embedded, publicShareId, token, setSessionToken])

  useEffect(() => {
    handleListRequestError(sharesRequestError, 'Failed to load shares.')
  }, [handleListRequestError, sharesRequestError])

  useEffect(() => {
    handleListRequestError(favoritesRequestError, 'Load favorites failed.')
  }, [favoritesRequestError, handleListRequestError])

  useEffect(() => {
    handleListRequestError(recentRequestError, 'Load recent files failed.')
  }, [handleListRequestError, recentRequestError])

  useEffect(() => {
    handleListRequestError(trashRequestError, 'Load recycle bin failed.')
  }, [handleListRequestError, trashRequestError])

  const loadDirectory = useCallback(
    async (path: string, authToken: string) => {
      setLoading(true)
      try {
        const response = await fetch(`/api/resources${encodePath(path)}`, {
          headers: withAuthHeaders(authToken),
        })

        if (response.status === 401) {
          setSessionToken('')
          setMessage('会话已失效，请在 Control Panel 重新登录。')
          return
        }

        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          setMessage(payload.error ?? `Failed to load directory (${response.status})`)
          return
        }

        const payload = (await response.json()) as DirectoryResponse | FileResponse
        const normalizedPath = normalizeUrlPath(payload.path || path)

        if (payload.is_dir) {
          setCurrentPathIsDir(true)
          setItems(Array.isArray((payload as DirectoryResponse).items) ? (payload as DirectoryResponse).items : [])
          clearPreviewState()
          setCurrentPath(normalizedPath)
          setSelectedPaths([])
          setMessage('')
          return
        }

        const entry: FileEntry = {
          name: fileNameFromPath(normalizedPath),
          path: normalizedPath,
          is_dir: false,
          size: (payload as FileResponse).size ?? 0,
          modified: (payload as FileResponse).modified ?? 0,
        }
        const kind = getFilePreviewKind(entry)

        setCurrentPathIsDir(false)
        setItems([])
        setCurrentPath(normalizedPath)
        setSelectedPaths([])
        setPreviewEntry(entry)
        setPreviewKind(kind)
        setPreviewError('')
        setPreviewLoading(false)
        setPreviewLoadingLabel('Loading preview...')
        setPreviewLoadingElapsedSeconds(0)
        setPreviewDocxHtml('')
        setPreviewDocPdfSrc((prev) => {
          revokeBlobUrl(prev)
          return ''
        })
        if (kind === 'text') {
          const text = (payload as FileResponse).content
          if (typeof text === 'string') {
            setPreviewTextContent(text)
          } else {
            setPreviewTextContent('')
            setPreviewError('This document preview is unavailable.')
          }
        } else {
          setPreviewTextContent('')
        }
        setMessage('')
      } finally {
        setLoading(false)
      }
    },
    [clearPreviewState, setSessionToken],
  )

  const loadShares = useCallback(
    async (authToken: string) => {
      void authToken
      await mutateShares()
    },
    [mutateShares],
  )

  const loadFavorites = useCallback(
    async (authToken: string) => {
      void authToken
      await mutateFavorites()
    },
    [mutateFavorites],
  )

  const loadRecent = useCallback(
    async (authToken: string) => {
      void authToken
      await mutateRecent()
    },
    [mutateRecent],
  )

  const loadTrash = useCallback(
    async (authToken: string) => {
      void authToken
      await mutateTrash()
    },
    [mutateTrash],
  )

  const toggleFavorite = useCallback(
    async (entry: FileEntry) => {
      const currentlyStarred = favoriteItems.some((item) => item.path === entry.path)
      const response = await fetch(`/api/favorites${currentlyStarred ? `?path=${encodeURIComponent(entry.path)}` : ''}`, {
        method: currentlyStarred ? 'DELETE' : 'POST',
        headers: withAuthHeaders(effectiveToken, {
          'Content-Type': 'application/json',
        }),
        body: currentlyStarred ? undefined : JSON.stringify({ path: entry.path }),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `${currentlyStarred ? 'Unstar' : 'Star'} failed (${response.status})`)
        return
      }

      await loadFavorites(effectiveToken)
      setMessage(currentlyStarred ? `Removed star: ${entry.name}` : `Starred: ${entry.name}`)
    },
    [effectiveToken, favoriteItems, loadFavorites],
  )

  const restoreTrashItem = useCallback(
    async (item: RecycleBinEntry) => {
      const response = await fetch('/api/recycle-bin/restore', {
        method: 'POST',
        headers: withAuthHeaders(effectiveToken, {
          'Content-Type': 'application/json',
        }),
        body: JSON.stringify({ item_id: item.item_id }),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `Restore failed (${response.status})`)
        return
      }

      await loadTrash(effectiveToken)
      await loadDirectory(currentPath, effectiveToken)
      setSelectedPaths((prev) => prev.filter((value) => value !== item.path))
      setMessage(`Restored: ${item.name}`)
    },
    [currentPath, effectiveToken, loadDirectory, loadTrash],
  )

  const deleteTrashItemForever = useCallback(
    (item: RecycleBinEntry) => {
      setConfirmDialog({
        title: 'Delete forever?',
        description: (
          <p>
            Permanently remove <span className="font-semibold text-slate-800">{item.name}</span> from recycle bin.
            This cannot be undone.
          </p>
        ),
        confirmLabel: 'Delete forever',
        confirmTone: 'danger',
        onConfirm: async () => {
          const response = await fetch(`/api/recycle-bin/item/${encodeURIComponent(item.item_id)}`, {
            method: 'DELETE',
            headers: withAuthHeaders(effectiveToken),
          })
          if (!response.ok) {
            const payload = (await response.json().catch(() => ({}))) as { error?: string }
            setMessage(payload.error ?? `Permanent delete failed (${response.status})`)
            return
          }

          await loadTrash(effectiveToken)
          setSelectedPaths((prev) => prev.filter((value) => value !== item.path))
          setMessage(`Deleted permanently: ${item.name}`)
        },
      })
    },
    [effectiveToken, loadTrash],
  )

  const updateUploadProgress = useCallback(
    (next: UploadProgressItem) => {
      setUploadProgress((prev) => {
        const index = prev.findIndex((item) => item.key === next.key)
        if (index < 0) {
          return [...prev, next]
        }
        const updated = [...prev]
        updated[index] = next
        return updated
      })
    },
    [setUploadProgress],
  )

  const patchUploadProgress = useCallback(
    (key: string, patch: Partial<UploadProgressItem>) => {
      setUploadProgress((prev) => prev.map((item) => (item.key === key ? { ...item, ...patch } : item)))
    },
    [setUploadProgress],
  )

  const loadPublicShare = useCallback(async (shareId: string, passwordInput: string, path = '/') => {
    if (!shareId) {
      return
    }

    setPublicShareLoading(true)
    setPublicShareError('')
    try {
      const response = await fetch(buildPublicShareApiPath(shareId, path, passwordInput))

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setPublicShareData(null)
        setPublicShareError(payload.error ?? `Load share failed (${response.status})`)
        return
      }

      const payload = (await response.json()) as PublicShareResponse
      const effectivePath = normalizeUrlPath(payload.path || path || '/')
      setPublicSharePath(effectivePath)
      syncPublicSharePathToUrl(effectivePath, passwordInput)
      setPublicPreviewExpanded(false)
      setPublicShareData(payload)
    } finally {
      setPublicShareLoading(false)
    }
  }, [syncPublicSharePathToUrl])

  const canNavigateUpByMouseBack = useCallback(() => {
    if (publicShareId) {
      return normalizeUrlPath(publicSharePath) !== '/'
    }

    if (mainTab !== 'files' || filesScope !== 'browse') {
      return false
    }

    return normalizeUrlPath(currentPath) !== '/'
  }, [currentPath, filesScope, mainTab, publicShareId, publicSharePath])

  const navigateUpByMouseBack = useCallback(() => {
    if (!canNavigateUpByMouseBack()) {
      return false
    }

    if (publicShareId) {
      const normalized = normalizeUrlPath(publicSharePath)
      void loadPublicShare(publicShareId, publicSharePassword, parentPath(normalized))
      return true
    }

    const normalized = normalizeUrlPath(currentPath)
    openDirectoryInBrowse(parentPath(normalized))
    return true
  }, [
    canNavigateUpByMouseBack,
    currentPath,
    loadPublicShare,
    openDirectoryInBrowse,
    publicShareId,
    publicSharePassword,
    publicSharePath,
  ])

  useEffect(() => {
    if (!effectiveToken || publicShareId || mainTab !== 'files' || filesScope !== 'browse') {
      return
    }
    void loadDirectory(currentPath, effectiveToken)
  }, [currentPath, effectiveToken, filesScope, loadDirectory, mainTab, publicShareId])

  useEffect(() => {
    if (!publicShareId && mainTab === 'files') {
      syncMainPathToUrl(currentPath)
    }
  }, [currentPath, publicShareId, mainTab, syncMainPathToUrl])

  useEffect(() => {
    if (embedded) {
      return
    }
    if (publicShareId) {
      return
    }
    const expected = getMainTabPath(mainTab)
    if (locationPathname !== expected) {
      setMainTab(getMainTabFromPathname(locationPathname))
    }
  }, [embedded, locationPathname, mainTab, publicShareId])

  useEffect(() => {
    setSelectedPaths([])
    clearSearchState()
    setMessage('')
    setDropzoneActive(false)
    dropDragDepthRef.current = 0
  }, [clearSearchState, filesScope])

  useEffect(() => {
    if (!publicShareId) {
      return
    }
    const initialPath = normalizeUrlPath(getSearchParam('path') || '/')
    const initialPassword = getSearchParam('password')
    void loadPublicShare(publicShareId, initialPassword, initialPath)
  }, [publicShareId, loadPublicShare])

  useEffect(() => {
    closeRowActionMenu()
  }, [currentPath, mainTab, searchActive, closeRowActionMenu])

  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null
      if (!target?.closest('[data-row-actions="true"]')) {
        closeRowActionMenu()
      }
    }
    window.addEventListener('pointerdown', onPointerDown)
    return () => {
      window.removeEventListener('pointerdown', onPointerDown)
    }
  }, [closeRowActionMenu])

  useEffect(() => {
    if (!openActionPath) {
      return
    }

    const handleDismiss = () => {
      closeRowActionMenu()
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        closeRowActionMenu()
      }
    }

    window.addEventListener('scroll', handleDismiss, true)
    window.addEventListener('resize', handleDismiss)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('scroll', handleDismiss, true)
      window.removeEventListener('resize', handleDismiss)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [openActionPath, closeRowActionMenu])

  useEffect(() => {
    if (embedded) {
      return
    }
    const onPopState = () => {
      const pathname = window.location.pathname
      setLocationPathname(pathname)
      const shareId = getPublicShareIdFromPath(pathname)
      const nextPath = normalizeUrlPath(getSearchParam('path') || '/')
      if (shareId) {
        const nextPassword = getSearchParam('password')
        setPublicSharePassword(nextPassword)
        void loadPublicShare(shareId, nextPassword, nextPath)
        return
      }

      setMainTab(getMainTabFromPathname(pathname))
      clearSearchState()
      setCurrentPath(nextPath)
    }

    window.addEventListener('popstate', onPopState)
    return () => {
      window.removeEventListener('popstate', onPopState)
    }
  }, [clearSearchState, embedded, loadPublicShare])

  useEffect(() => {
    const onMouseDown = (event: MouseEvent) => {
      if (event.button !== 3) {
        return
      }
      if (!canNavigateUpByMouseBack()) {
        return
      }
      event.preventDefault()
      event.stopPropagation()
    }

    const onMouseUp = (event: MouseEvent) => {
      if (event.button !== 3) {
        return
      }
      if (!navigateUpByMouseBack()) {
        return
      }
      event.preventDefault()
      event.stopPropagation()
    }

    const onAuxClick = (event: MouseEvent) => {
      if (event.button !== 3) {
        return
      }
      if (!navigateUpByMouseBack()) {
        return
      }
      event.preventDefault()
      event.stopPropagation()
    }

    window.addEventListener('mousedown', onMouseDown, true)
    window.addEventListener('mouseup', onMouseUp, true)
    window.addEventListener('auxclick', onAuxClick, true)
    return () => {
      window.removeEventListener('mousedown', onMouseDown, true)
      window.removeEventListener('mouseup', onMouseUp, true)
      window.removeEventListener('auxclick', onAuxClick, true)
    }
  }, [canNavigateUpByMouseBack, navigateUpByMouseBack])

  useEffect(() => {
    uploadPausedRef.current = uploadPaused
  }, [uploadPaused])

  useEffect(() => {
    if (!imageViewerOpen) {
      return
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        closeImageViewer()
      } else if (event.key === '+' || event.key === '=') {
        event.preventDefault()
        zoomInImage()
      } else if (event.key === '-') {
        event.preventDefault()
        zoomOutImage()
      } else if (event.key === '0') {
        event.preventDefault()
        resetImageZoom()
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [closeImageViewer, imageViewerOpen, resetImageZoom, zoomInImage, zoomOutImage])

  const onSearch = async () => {
    const keyword = searchKeyword.trim()
    if (!keyword) {
      setMessage('Please enter a keyword to search.')
      return
    }

    if (filesScope !== 'browse') {
      const lowered = keyword.toLowerCase()
      const localBase =
        filesScope === 'recent'
          ? recentItems.map((item) => ({
              name: item.name,
              path: item.path,
              is_dir: item.is_dir,
              size: item.size,
              modified: item.modified,
            }))
          : filesScope === 'starred'
            ? favoriteItems
            : filesScope === 'trash'
              ? trashItems.map((item) => ({
                  name: item.name,
                  path: item.path,
                  is_dir: item.is_dir,
                  size: item.size,
                  modified: item.modified,
                }))
              : items

      const resultItems = localBase.filter((item) => {
        const hitName = item.name.toLowerCase().includes(lowered)
        const hitPath = item.path.toLowerCase().includes(lowered)
        return hitName || hitPath
      })
      setSearchResults(resultItems)
      setSearchTruncated(false)
      setSearchActive(true)
      setSelectedPaths([])
      setMessage(`Found ${resultItems.length} result(s).`)
      return
    }

    setSearchLoading(true)
    try {
      const query = new URLSearchParams({
        q: keyword,
        path: currentPath,
        kind: 'all',
        limit: '200',
      })
      const response = await fetch(`/api/search?${query.toString()}`, {
        headers: withAuthHeaders(effectiveToken),
      })

      if (response.status === 401) {
        setSessionToken('')
        setMessage('会话已失效，请在 Control Panel 重新登录。')
        return
      }

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `Search failed (${response.status})`)
        return
      }

      const payload = (await response.json()) as SearchResponse
      const resultItems = Array.isArray(payload.items) ? payload.items : []
      setSearchResults(resultItems)
      setSearchTruncated(Boolean(payload.truncated))
      setSearchActive(true)
      setSelectedPaths([])
      setMessage(`Found ${resultItems.length} result(s) in ${payload.path}.`)
    } finally {
      setSearchLoading(false)
    }
  }

  const onClearSearch = () => {
    clearSearchState()
    setSelectedPaths([])
    setMessage('')
  }

  const onClearCompletedUploads = () => {
    setUploadProgress((prev) =>
      prev.filter(
        (item) =>
          item.status === 'uploading' ||
          item.status === 'paused' ||
          item.status === 'error' ||
          item.status === 'cancelled',
      ),
    )
  }

  const isSelected = useCallback((path: string) => selectedPaths.includes(path), [selectedPaths])

  const toggleSelection = (path: string) => {
    setSelectedPaths((prev) => (prev.includes(path) ? prev.filter((item) => item !== path) : [...prev, path]))
  }

  const onIconEntryClick = useCallback((event: ReactMouseEvent<HTMLElement>, entry: FileEntry) => {
    const target = event.target as HTMLElement
    if (target.closest('[data-row-actions="true"]')) {
      return
    }

    if (event.metaKey || event.ctrlKey) {
      setSelectedPaths((prev) => (prev.includes(entry.path) ? prev.filter((item) => item !== entry.path) : [...prev, entry.path]))
      return
    }

    setSelectedPaths([entry.path])
  }, [])

  const onIconEntryDoubleClick = useCallback(
    (event: ReactMouseEvent<HTMLElement>, entry: FileEntry) => {
      const target = event.target as HTMLElement
      if (target.closest('[data-row-actions="true"]')) {
        return
      }

      if (filesScope === 'trash') {
        return
      }

      if (entry.is_dir) {
        openDirectoryInBrowse(entry.path)
        return
      }

      window.open(buildFileDetailPath(entry.path), '_blank', 'noopener,noreferrer')
    },
    [filesScope, openDirectoryInBrowse],
  )

  const scopeItems = useMemo<FileEntry[]>(() => {
    if (searchActive) {
      return searchResults
    }
    if (filesScope === 'recent') {
      return recentItems.map((item) => ({
        name: item.name,
        path: item.path,
        is_dir: item.is_dir,
        size: item.size,
        modified: item.modified,
      }))
    }
    if (filesScope === 'starred') {
      return favoriteItems
    }
    if (filesScope === 'trash') {
      return trashItems.map((item) => ({
        name: item.name,
        path: item.path,
        is_dir: item.is_dir,
        size: item.size,
        modified: item.modified,
      }))
    }
    return items
  }, [favoriteItems, filesScope, items, recentItems, searchActive, searchResults, trashItems])

  const visibleItems = useMemo(() => {
    const extensions = filterExtInput
      .split(',')
      .map((value) => value.trim().replace(/^\./, '').toLowerCase())
      .filter(Boolean)

    const nowSec = Math.floor(Date.now() / 1000)
    const presetFrom =
      filterDatePreset === '7d'
        ? nowSec - 7 * 24 * 3600
        : filterDatePreset === '30d'
          ? nowSec - 30 * 24 * 3600
          : null
    const customFrom = filterDateFrom ? Math.floor(new Date(filterDateFrom).getTime() / 1000) : null
    const customTo = filterDateTo ? Math.floor(new Date(filterDateTo).getTime() / 1000) : null
    const effectiveFrom = filterDatePreset === 'custom' ? customFrom : presetFrom
    const effectiveTo = filterDatePreset === 'custom' ? customTo : null

    const sizeMinMb = Number(filterSizeMin)
    const sizeMaxMb = Number(filterSizeMax)
    const sizeMin = Number.isFinite(sizeMinMb) && sizeMinMb > 0 ? Math.floor(sizeMinMb * 1024 * 1024) : null
    const sizeMax = Number.isFinite(sizeMaxMb) && sizeMaxMb > 0 ? Math.floor(sizeMaxMb * 1024 * 1024) : null

    const filtered = scopeItems.filter((entry) => {
      if (filterKind === 'file' && entry.is_dir) {
        return false
      }
      if (filterKind === 'dir' && !entry.is_dir) {
        return false
      }

      if (extensions.length > 0) {
        if (entry.is_dir) {
          return false
        }
        const ext = getFileExtension(entry.name).toLowerCase()
        if (!extensions.includes(ext)) {
          return false
        }
      }

      if (effectiveFrom != null && Number.isFinite(effectiveFrom) && entry.modified < effectiveFrom) {
        return false
      }
      if (effectiveTo != null && Number.isFinite(effectiveTo) && entry.modified > effectiveTo) {
        return false
      }

      if (!entry.is_dir) {
        if (sizeMin != null && entry.size < sizeMin) {
          return false
        }
        if (sizeMax != null && entry.size > sizeMax) {
          return false
        }
      }

      return true
    })

    filtered.sort((a, b) => {
      if (a.is_dir !== b.is_dir) {
        return a.is_dir ? -1 : 1
      }

      const base =
        filterSortBy === 'modified'
          ? a.modified - b.modified
          : filterSortBy === 'size'
            ? a.size - b.size
            : a.name.toLowerCase().localeCompare(b.name.toLowerCase())
      if (base === 0) {
        return a.path.toLowerCase().localeCompare(b.path.toLowerCase())
      }
      return base
    })

    if (filterSortOrder === 'desc') {
      filtered.reverse()
    }
    return filtered
  }, [
    filterDateFrom,
    filterDatePreset,
    filterDateTo,
    filterExtInput,
    filterKind,
    filterSizeMax,
    filterSizeMin,
    filterSortBy,
    filterSortOrder,
    scopeItems,
  ])

  useEffect(() => {
    if (!openActionPath) {
      return
    }
    if (!visibleItems.some((entry) => entry.path === openActionPath)) {
      closeRowActionMenu()
    }
  }, [openActionPath, visibleItems, closeRowActionMenu])

  const allSelected = visibleItems.length > 0 && visibleItems.every((item) => selectedPaths.includes(item.path))

  const toggleSelectAll = () => {
    if (allSelected) {
      setSelectedPaths([])
      return
    }
    setSelectedPaths(visibleItems.map((item) => item.path))
  }

  const selectedEntries = useMemo(
    () => visibleItems.filter((item) => selectedPaths.includes(item.path)),
    [visibleItems, selectedPaths],
  )

  const sleep = useCallback((ms: number) => new Promise((resolve) => setTimeout(resolve, ms)), [])

  const createUploadSession = useCallback(
    async (authToken: string, targetPath: string, size: number, chunkSize: number) => {
      const response = await fetch('/api/upload/session', {
        method: 'POST',
        headers: withAuthHeaders(authToken, {
          'Content-Type': 'application/json',
        }),
        body: JSON.stringify({
          path: targetPath,
          size,
          chunk_size: chunkSize,
          override_existing: true,
        }),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        throw new Error(payload.error ?? `Create upload session failed (${response.status})`)
      }

      const payload = (await response.json()) as { session?: UploadSessionRecord }
      if (!payload.session) {
        throw new Error('Create upload session failed: invalid response')
      }
      return payload.session
    },
    [],
  )

  const getUploadSession = useCallback(async (authToken: string, sessionId: string) => {
    const response = await fetch(`/api/upload/session/${encodeURIComponent(sessionId)}`, {
      headers: withAuthHeaders(authToken),
    })
    if (!response.ok) {
      return null
    }
    const payload = (await response.json()) as { session?: UploadSessionRecord }
    return payload.session ?? null
  }, [])

  const uploadSessionChunk = useCallback(
    async (authToken: string, sessionId: string, offset: number, chunk: Blob) => {
      const response = await fetch(`/api/upload/session/${encodeURIComponent(sessionId)}?offset=${offset}`, {
        method: 'PUT',
        headers: withAuthHeaders(authToken),
        body: chunk,
      })

      if (response.status === 409) {
        const payload = (await response.json().catch(() => ({}))) as { expected_offset?: number; error?: string }
        return {
          ok: false,
          expectedOffset: payload.expected_offset,
          error: payload.error ?? 'Chunk offset mismatch',
        }
      }

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        throw new Error(payload.error ?? `Upload chunk failed (${response.status})`)
      }

      const payload = (await response.json()) as { uploaded_size?: number }
      return {
        ok: true,
        uploadedSize: payload.uploaded_size ?? offset + chunk.size,
      }
    },
    [],
  )

  const completeUploadSession = useCallback(async (authToken: string, sessionId: string) => {
    const response = await fetch(`/api/upload/session/${encodeURIComponent(sessionId)}/complete`, {
      method: 'POST',
      headers: withAuthHeaders(authToken),
    })
    if (!response.ok) {
      const payload = (await response.json().catch(() => ({}))) as { error?: string }
      throw new Error(payload.error ?? `Complete upload failed (${response.status})`)
    }
  }, [])

  const deleteUploadSession = useCallback(async (authToken: string, sessionId: string) => {
    const response = await fetch(`/api/upload/session/${encodeURIComponent(sessionId)}`, {
      method: 'DELETE',
      headers: withAuthHeaders(authToken),
    })

    if (response.status === 404) {
      return
    }
    if (!response.ok) {
      const payload = (await response.json().catch(() => ({}))) as { error?: string }
      throw new Error(payload.error ?? `Delete upload session failed (${response.status})`)
    }
  }, [])

  const performUploadFile = useCallback(
    async (
      authToken: string,
      file: File,
      targetPath: string,
      progressKey: string,
      resumeKey: string,
    ): Promise<{ ok: boolean; cancelled: boolean; error?: string }> => {
      const waitForResume = async (offset: number) => {
        while (uploadPausedRef.current) {
          updateUploadProgress({
            key: progressKey,
            name: file.name,
            uploaded: offset,
            total: file.size,
            status: 'paused',
          })
          await sleep(200)
        }
      }

      const cancelCurrentUpload = async (sessionId?: string) => {
        if (sessionId) {
          try {
            await deleteUploadSession(authToken, sessionId)
          } catch {
            // ignore cleanup errors
          }
        }
        localStorage.removeItem(resumeKey)
        uploadSessionRef.current.delete(progressKey)
      }

      try {
        const expectedChunkSize = Math.min(DEFAULT_UPLOAD_CHUNK_SIZE, Math.max(file.size, 64 * 1024))
        let sessionId = localStorage.getItem(resumeKey) ?? ''
        let session: UploadSessionRecord | null = null

        if (sessionId) {
          session = await getUploadSession(authToken, sessionId)
          if (!session || session.path !== targetPath || session.size !== file.size) {
            session = null
            sessionId = ''
            localStorage.removeItem(resumeKey)
          }
        }

        if (!session) {
          session = await createUploadSession(authToken, targetPath, file.size, expectedChunkSize)
          sessionId = session.id
          localStorage.setItem(resumeKey, sessionId)
        }

        uploadSessionRef.current.set(progressKey, session.id)

        let offset = Math.min(session.uploaded_size, file.size)
        updateUploadProgress({
          key: progressKey,
          name: file.name,
          uploaded: offset,
          total: file.size,
          status: 'uploading',
        })

        const chunkSize = Math.max(64 * 1024, session.chunk_size || expectedChunkSize)
        while (offset < file.size) {
          if (uploadCancelledRef.current.has(progressKey)) {
            await cancelCurrentUpload(session.id)
            updateUploadProgress({
              key: progressKey,
              name: file.name,
              uploaded: offset,
              total: file.size,
              status: 'cancelled',
              error: 'Cancelled by user',
            })
            return { ok: false, cancelled: true, error: 'Cancelled by user' }
          }

          await waitForResume(offset)
          if (uploadCancelledRef.current.has(progressKey)) {
            await cancelCurrentUpload(session.id)
            updateUploadProgress({
              key: progressKey,
              name: file.name,
              uploaded: offset,
              total: file.size,
              status: 'cancelled',
              error: 'Cancelled by user',
            })
            return { ok: false, cancelled: true, error: 'Cancelled by user' }
          }

          updateUploadProgress({
            key: progressKey,
            name: file.name,
            uploaded: offset,
            total: file.size,
            status: 'uploading',
          })

          const nextOffset = Math.min(offset + chunkSize, file.size)
          const chunk = file.slice(offset, nextOffset)

          let attempt = 0
          let uploaded = false
          while (!uploaded) {
            if (uploadCancelledRef.current.has(progressKey)) {
              await cancelCurrentUpload(session.id)
              updateUploadProgress({
                key: progressKey,
                name: file.name,
                uploaded: offset,
                total: file.size,
                status: 'cancelled',
                error: 'Cancelled by user',
              })
              return { ok: false, cancelled: true, error: 'Cancelled by user' }
            }

            try {
              const result = await uploadSessionChunk(authToken, session.id, offset, chunk)
              if (!result.ok) {
                if (typeof result.expectedOffset === 'number' && result.expectedOffset >= 0) {
                  offset = Math.min(result.expectedOffset, file.size)
                  uploaded = true
                  continue
                }
                throw new Error(result.error ?? 'Upload chunk failed')
              }
              offset = Math.min(result.uploadedSize ?? nextOffset, file.size)
              uploaded = true
            } catch (error) {
              attempt += 1
              if (attempt > UPLOAD_MAX_RETRY) {
                throw error
              }
              await sleep(UPLOAD_RETRY_BASE_DELAY_MS * attempt)
            }
          }
        }

        await waitForResume(file.size)
        if (uploadCancelledRef.current.has(progressKey)) {
          await cancelCurrentUpload(session.id)
          updateUploadProgress({
            key: progressKey,
            name: file.name,
            uploaded: file.size,
            total: file.size,
            status: 'cancelled',
            error: 'Cancelled by user',
          })
          return { ok: false, cancelled: true, error: 'Cancelled by user' }
        }

        await completeUploadSession(authToken, session.id)
        localStorage.removeItem(resumeKey)
        uploadSessionRef.current.delete(progressKey)
        updateUploadProgress({
          key: progressKey,
          name: file.name,
          uploaded: file.size,
          total: file.size,
          status: 'completed',
        })
        return { ok: true, cancelled: false }
      } catch (error) {
        const message = error instanceof Error ? error.message : `Upload failed for ${file.name}`
        patchUploadProgress(progressKey, {
          status: 'error',
          error: message,
        })
        return { ok: false, cancelled: false, error: message }
      } finally {
        uploadCancelledRef.current.delete(progressKey)
      }
    },
    [
      completeUploadSession,
      createUploadSession,
      deleteUploadSession,
      getUploadSession,
      patchUploadProgress,
      sleep,
      updateUploadProgress,
      uploadSessionChunk,
    ],
  )

  const uploadFilesToCurrentPath = async (files: File[]) => {
    if (files.length === 0) {
      return
    }

    setLoading(true)
    setUploadPaused(false)
    setUploadPanelOpen(true)
    uploadPausedRef.current = false
    setDropzoneActive(false)
    dropDragDepthRef.current = 0
    try {
      const base = normalizeDirPath(currentPath)
      let completedCount = 0
      let cancelledCount = 0
      const errors: string[] = []

      for (const file of files) {
        const targetPath = `${base}${file.name}`
        const progressKey = `${targetPath}:${file.size}:${file.lastModified}`
        const resumeKey = getUploadResumeKey(targetPath, file.size, file.lastModified)
        uploadFilesRef.current.set(progressKey, { file, targetPath, resumeKey })
      }

      let queueIndex = 0
      const workerCount = Math.max(1, Math.min(UPLOAD_CONCURRENCY, files.length))
      const workers = Array.from({ length: workerCount }, async () => {
        while (queueIndex < files.length) {
          const current = queueIndex
          queueIndex += 1
          const currentFile = files[current]
          if (!currentFile) {
            continue
          }
          const targetPath = `${base}${currentFile.name}`
          const progressKey = `${targetPath}:${currentFile.size}:${currentFile.lastModified}`
          const resumeKey = getUploadResumeKey(targetPath, currentFile.size, currentFile.lastModified)
          const result = await performUploadFile(effectiveToken, currentFile, targetPath, progressKey, resumeKey)
          if (result.ok) {
            completedCount += 1
          } else if (result.cancelled) {
            cancelledCount += 1
          } else {
            errors.push(`${currentFile.name}: ${result.error ?? 'Upload failed'}`)
          }
        }
      })

      await Promise.all(workers)

      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      if (completedCount === files.length) {
        setMessage(`Upload completed (${completedCount}/${files.length}).`)
      } else {
        const detail = errors.length > 0 ? ` Failed: ${errors.slice(0, 2).join(' | ')}` : ''
        const cancelledDetail = cancelledCount > 0 ? ` Cancelled: ${cancelledCount}.` : ''
        setMessage(`Upload partially completed (${completedCount}/${files.length}).${cancelledDetail}${detail}`)
      }
    } finally {
      setUploadPaused(false)
      uploadPausedRef.current = false
      setLoading(false)
    }
  }

  const onUpload: ChangeEventHandler<HTMLInputElement> = async (event) => {
    const selected = event.target.files
    if (!selected || selected.length === 0) {
      return
    }

    try {
      await uploadFilesToCurrentPath(Array.from(selected))
    } finally {
      event.target.value = ''
    }
  }

  const onListDragEnter = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    event.stopPropagation()
    dropDragDepthRef.current += 1
    setDropzoneActive(true)
  }

  const onListDragOver = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    event.stopPropagation()
    event.dataTransfer.dropEffect = 'copy'
    if (!dropzoneActive) {
      setDropzoneActive(true)
    }
  }

  const onListDragLeave = (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    event.stopPropagation()
    dropDragDepthRef.current = Math.max(0, dropDragDepthRef.current - 1)
    if (dropDragDepthRef.current === 0) {
      setDropzoneActive(false)
    }
  }

  const onListDrop = async (event: DragEvent<HTMLDivElement>) => {
    event.preventDefault()
    event.stopPropagation()
    dropDragDepthRef.current = 0
    setDropzoneActive(false)
    const droppedFiles = Array.from(event.dataTransfer.files ?? [])
    if (droppedFiles.length === 0) {
      return
    }

    await uploadFilesToCurrentPath(droppedFiles)
  }

  const onCancelUpload = async (key: string) => {
    uploadCancelledRef.current.add(key)
    patchUploadProgress(key, {
      status: 'cancelled',
      error: 'Cancelled by user',
    })

    const sessionId = uploadSessionRef.current.get(key)
    if (sessionId) {
      try {
        await deleteUploadSession(effectiveToken, sessionId)
      } catch {
        // ignore cleanup error
      }
      uploadSessionRef.current.delete(key)
    }

    const task = uploadFilesRef.current.get(key)
    if (task) {
      localStorage.removeItem(task.resumeKey)
    }
  }

  const onRetryUpload = async (key: string) => {
    const task = uploadFilesRef.current.get(key)
    if (!task) {
      setMessage('Cannot retry this upload item. Please select the file again.')
      return
    }

    uploadCancelledRef.current.delete(key)
    patchUploadProgress(key, {
      uploaded: 0,
      status: 'uploading',
      error: '',
    })

    setLoading(true)
    try {
      const result = await performUploadFile(effectiveToken, task.file, task.targetPath, key, task.resumeKey)
      if (result.ok) {
        clearSearchState()
        await loadDirectory(currentPath, effectiveToken)
        setMessage(`Retry succeeded: ${task.file.name}`)
      } else if (result.cancelled) {
        setMessage(`Retry cancelled: ${task.file.name}`)
      } else {
        setMessage(result.error ?? `Retry failed: ${task.file.name}`)
      }
    } finally {
      setLoading(false)
    }
  }

  const onDelete = (entry: FileEntry) => {
    setConfirmDialog({
      title: `Delete ${entry.is_dir ? 'folder' : 'file'}?`,
      description: (
        <p>
          <span className="font-semibold text-slate-800">{entry.name}</span> will be moved to recycle bin.
        </p>
      ),
      confirmLabel: 'Move to trash',
      confirmTone: 'danger',
      onConfirm: async () => {
        setLoading(true)
        try {
          const response = await fetch(`/api/resources${encodePath(entry.path)}`, {
            method: 'DELETE',
            headers: withAuthHeaders(effectiveToken),
          })

          if (!response.ok) {
            const payload = (await response.json().catch(() => ({}))) as { error?: string }
            setMessage(payload.error ?? `Delete failed (${response.status})`)
            return
          }

          setSelectedPaths((prev) => prev.filter((item) => item !== entry.path))
          if (editorPath === entry.path) {
            setEditorPath('')
            setEditorContent('')
            setEditorDirty(false)
          }

          clearSearchState()
          if (filesScope === 'browse') {
            await loadDirectory(currentPath, effectiveToken)
          } else if (filesScope === 'recent') {
            await loadRecent(effectiveToken)
          } else if (filesScope === 'starred') {
            await loadFavorites(effectiveToken)
          }
          await loadTrash(effectiveToken)
          const successMessage = entry.is_dir ? `Moved folder to trash: ${entry.name}` : `Moved file to trash: ${entry.name}`
          setMessage(successMessage)
          showDeleteToast(successMessage)
        } finally {
          setLoading(false)
        }
      },
    })
  }

  const onCreateShare = (entry: FileEntry) => {
    setShareDialog({
      entry,
      expiresInSeconds: '86400',
      password: '',
      error: '',
    })
  }

  const onDeleteShare = (shareId: string) => {
    const targetShare = shares.find((item) => item.id === shareId)
    const label = targetShare?.path ?? 'this share link'
    setConfirmDialog({
      title: 'Remove share link?',
      description: (
        <p>
          Remove share link for <span className="font-semibold text-slate-800">{label}</span>.
        </p>
      ),
      confirmLabel: 'Remove',
      confirmTone: 'danger',
      onConfirm: async () => {
        setLoading(true)
        try {
          const response = await fetch(`/api/share/${encodeURIComponent(shareId)}`, {
            method: 'DELETE',
            headers: withAuthHeaders(effectiveToken),
          })

          if (!response.ok) {
            const payload = (await response.json().catch(() => ({}))) as { error?: string }
            setMessage(payload.error ?? `Delete share failed (${response.status})`)
            return
          }

          await loadShares(effectiveToken)
          setMessage('Share removed.')
        } finally {
          setLoading(false)
        }
      },
    })
  }

  const onCopyShareLink = async (shareId: string, type: 'view' | 'download') => {
    const path = type === 'view' ? buildPublicSharePath(shareId) : buildPublicDownloadPath(shareId)
    const absolute = `${window.location.origin}${path}`
    try {
      await navigator.clipboard.writeText(absolute)
      setMessage(`${type === 'view' ? 'View' : 'Download'} link copied.`)
    } catch {
      setMessage(`Copy failed. Link: ${absolute}`)
    }
  }

  const patchResource = async (
    sourcePath: string,
    payload: {
      action: 'move' | 'copy'
      destination: string
      override_existing?: boolean
    },
  ) =>
    fetch(`/api/resources${encodePath(sourcePath)}`, {
      method: 'PATCH',
      headers: withAuthHeaders(effectiveToken, {
        'Content-Type': 'application/json',
      }),
      body: JSON.stringify(payload),
    })

  const onMoveOrCopy = (entry: FileEntry, action: 'move' | 'copy') => {
    const suggested = action === 'copy' ? `${entry.path}.copy` : entry.path
    setMoveCopyDialog({
      mode: 'single',
      action,
      sourceEntry: entry,
      destination: suggested,
      overrideExisting: false,
      error: '',
    })
  }

  const onBatchDelete = () => {
    if (selectedEntries.length === 0) {
      return
    }

    const total = selectedEntries.length
    setConfirmDialog({
      title: 'Delete selected items?',
      description: <p>{`Move ${total} selected item(s) to recycle bin.`}</p>,
      confirmLabel: 'Move to trash',
      confirmTone: 'danger',
      onConfirm: async () => {
        setLoading(true)
        try {
          for (const entry of selectedEntries) {
            const response = await fetch(`/api/resources${encodePath(entry.path)}`, {
              method: 'DELETE',
              headers: withAuthHeaders(effectiveToken),
            })
            if (!response.ok) {
              const payload = (await response.json().catch(() => ({}))) as { error?: string }
              setMessage(payload.error ?? `Batch delete failed at ${entry.name}`)
              return
            }
          }

          if (selectedPaths.includes(editorPath)) {
            setEditorPath('')
            setEditorContent('')
            setEditorDirty(false)
          }
          setSelectedPaths([])
          clearSearchState()
          await loadDirectory(currentPath, effectiveToken)
          const successMessage = `Moved ${total} item(s) to trash.`
          setMessage(successMessage)
          showDeleteToast(successMessage)
        } finally {
          setLoading(false)
        }
      },
    })
  }

  const onBatchMoveOrCopy = (action: 'move' | 'copy') => {
    if (selectedEntries.length === 0) {
      return
    }

    setMoveCopyDialog({
      mode: 'batch',
      action,
      sourceEntry: null,
      destination: currentPath,
      overrideExisting: false,
      error: '',
    })
  }

  const submitMoveCopyDialog = async () => {
    if (!moveCopyDialog) {
      return
    }

    const destination = moveCopyDialog.destination.trim()
    if (!destination) {
      setMoveCopyDialog((prev) => (prev ? { ...prev, error: 'Destination is required.' } : prev))
      return
    }

    if (moveCopyDialog.mode === 'single' && moveCopyDialog.sourceEntry && destination === moveCopyDialog.sourceEntry.path) {
      setMoveCopyDialog((prev) => (prev ? { ...prev, error: 'Destination must be different from source path.' } : prev))
      return
    }

    setMoveCopyDialogBusy(true)
    setLoading(true)
    try {
      if (moveCopyDialog.mode === 'single' && moveCopyDialog.sourceEntry) {
        const sourceEntry = moveCopyDialog.sourceEntry
        const response = await patchResource(sourceEntry.path, {
          action: moveCopyDialog.action,
          destination,
          override_existing: moveCopyDialog.overrideExisting,
        })

        if (response.status === 409 && !moveCopyDialog.overrideExisting) {
          setMoveCopyDialog((prev) =>
            prev ? { ...prev, error: 'Target already exists. Enable override and retry.' } : prev,
          )
          return
        }

        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          setMessage(payload.error ?? `${moveCopyDialog.action} failed (${response.status})`)
          return
        }

        if (moveCopyDialog.action === 'move' && editorPath === sourceEntry.path) {
          setEditorPath(destination)
        }
        setSelectedPaths((prev) => prev.filter((item) => item !== sourceEntry.path))

        clearSearchState()
        await loadDirectory(currentPath, effectiveToken)
        setMessage(moveCopyDialog.action === 'move' ? 'Item moved.' : 'Item copied.')
        setMoveCopyDialog(null)
        return
      }

      for (const entry of selectedEntries) {
        const entryDestination = joinPath(destination, fileNameFromPath(entry.path))
        const response = await patchResource(entry.path, {
          action: moveCopyDialog.action,
          destination: entryDestination,
          override_existing: moveCopyDialog.overrideExisting,
        })

        if (response.status === 409 && !moveCopyDialog.overrideExisting) {
          setMoveCopyDialog((prev) =>
            prev ? { ...prev, error: `Target exists for ${entry.name}. Enable override and retry.` } : prev,
          )
          return
        }

        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          setMessage(payload.error ?? `Batch ${moveCopyDialog.action} failed at ${entry.name}`)
          return
        }

        if (moveCopyDialog.action === 'move' && editorPath === entry.path) {
          setEditorPath(entryDestination)
        }
      }

      const total = selectedEntries.length
      setSelectedPaths([])
      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      setMessage(`${moveCopyDialog.action === 'move' ? 'Moved' : 'Copied'} ${total} item(s).`)
      setMoveCopyDialog(null)
    } finally {
      setLoading(false)
      setMoveCopyDialogBusy(false)
    }
  }

  const submitShareDialog = async () => {
    if (!shareDialog) {
      return
    }

    const expiresRaw = shareDialog.expiresInSeconds.trim()
    let expiresInSeconds: number | undefined
    if (expiresRaw) {
      const parsed = Number(expiresRaw)
      if (!Number.isFinite(parsed) || parsed <= 0) {
        setShareDialog((prev) => (prev ? { ...prev, error: 'Expiration must be a positive number.' } : prev))
        return
      }
      expiresInSeconds = Math.floor(parsed)
    }

    setShareDialogBusy(true)
    setLoading(true)
    try {
      const response = await fetch('/api/share', {
        method: 'POST',
        headers: withAuthHeaders(effectiveToken, {
          'Content-Type': 'application/json',
        }),
        body: JSON.stringify({
          path: shareDialog.entry.path,
          password: shareDialog.password.trim() || undefined,
          expires_in_seconds: expiresInSeconds,
        }),
      })

      if (response.status === 401) {
        setSessionToken('')
        setMessage('会话已失效，请在 Control Panel 重新登录。')
        return
      }

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        const messageText = payload.error ?? `Create share failed (${response.status})`
        setShareDialog((prev) => (prev ? { ...prev, error: messageText } : prev))
        setMessage(messageText)
        return
      }

      await loadShares(effectiveToken)
      setShareDialog(null)
      setMessage(`Share created for ${shareDialog.entry.name}`)
    } finally {
      setLoading(false)
      setShareDialogBusy(false)
    }
  }

  const onOpenEditor = async (entry: FileEntry) => {
    if (entry.is_dir) {
      return
    }

    setLoading(true)
    try {
      const response = await fetch(`/api/resources${encodePath(entry.path)}?content=1`, {
        headers: withAuthHeaders(effectiveToken),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `Open file failed (${response.status})`)
        return
      }

      const payload = (await response.json()) as FileResponse
      if (payload.is_dir) {
        setMessage('Cannot edit a folder.')
        return
      }
      if (payload.content == null) {
        setMessage('Only UTF-8 text files are editable in this version.')
        return
      }

      setEditorPath(payload.path)
      setEditorContent(payload.content)
      setEditorDirty(false)
      setMessage(`Editing ${payload.path}`)
      navigateToMainTab('editor')
    } finally {
      setLoading(false)
    }
  }

  const onSaveEditor = async () => {
    if (!editorPath) {
      return
    }

    setEditorSaving(true)
    try {
      const response = await fetch(`/api/resources${encodePath(editorPath)}`, {
        method: 'PUT',
        headers: withAuthHeaders(effectiveToken, {
          'Content-Type': 'application/json',
        }),
        body: JSON.stringify({
          content: editorContent,
        }),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `Save failed (${response.status})`)
        return
      }

      setEditorDirty(false)
      setMessage(`Saved ${editorPath}`)
      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
    } finally {
      setEditorSaving(false)
    }
  }

  const closeEditorNow = useCallback(() => {
    setEditorPath('')
    setEditorContent('')
    setEditorDirty(false)
    navigateToMainTab('files')
  }, [navigateToMainTab])

  const onCloseEditor = () => {
    if (editorDirty) {
      setConfirmDialog({
        title: 'Discard unsaved changes?',
        description: <p>Editor changes have not been saved.</p>,
        confirmLabel: 'Discard changes',
        confirmTone: 'danger',
        onConfirm: () => {
          closeEditorNow()
        },
      })
      return
    }
    closeEditorNow()
  }

  const onCreateFolder = () => {
    setFolderDialogOpen(true)
    setFolderNameInput('')
    setFolderDialogError('')
  }

  const submitCreateFolderDialog = async () => {
    const folderName = folderNameInput.trim()
    if (!folderName) {
      setFolderDialogError('Folder name is required.')
      return
    }
    if (folderName.includes('/')) {
      setFolderDialogError('Folder name cannot include "/".')
      return
    }

    setFolderDialogBusy(true)
    setLoading(true)
    try {
      const targetPath = `${joinPath(currentPath, folderName)}/`
      const response = await fetch(`/api/resources${encodePath(targetPath)}`, {
        method: 'POST',
        headers: withAuthHeaders(effectiveToken),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        const messageText = payload.error ?? `Create folder failed (${response.status})`
        setFolderDialogError(messageText)
        setMessage(messageText)
        return
      }

      closeFolderDialog(true)
      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      setMessage('Folder created.')
    } finally {
      setLoading(false)
      setFolderDialogBusy(false)
    }
  }

  const onSubmitRename = async () => {
    if (!renameTarget) {
      return
    }

    const currentEntry = renameTarget
    const nextName = currentEntry.is_dir
      ? renameBaseName.trim()
      : buildNameFromParts(renameBaseName, renameExtension)

    if (!nextName) {
      setRenameError('Name cannot be empty.')
      return
    }

    if (nextName.includes('/')) {
      setRenameError('Name cannot include "/".')
      return
    }

    if (nextName === currentEntry.name) {
      closeRenameModal()
      return
    }

    setRenameSubmitting(true)
    setLoading(true)
    try {
      const response = await fetch(`/api/resources${encodePath(currentEntry.path)}`, {
        method: 'PATCH',
        headers: withAuthHeaders(effectiveToken, {
          'Content-Type': 'application/json',
        }),
        body: JSON.stringify({
          action: 'rename',
          new_name: nextName,
        }),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        const errorText = payload.error ?? `Rename failed (${response.status})`
        setRenameError(errorText)
        setMessage(errorText)
        return
      }

      if (editorPath === currentEntry.path) {
        setEditorPath(renamePath(currentEntry.path, nextName))
      }

      closeRenameModal(true)
      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      setMessage('Item renamed.')
    } finally {
      setLoading(false)
      setRenameSubmitting(false)
    }
  }

  const downloadQuery = useMemo(() => encodeURIComponent(effectiveToken), [effectiveToken])
  const buildRawFileUrl = useCallback(
    (path: string, forceDownload = false) =>
      `/api/raw${encodePath(path)}?auth=${downloadQuery}${forceDownload ? '&download=1' : ''}`,
    [downloadQuery],
  )
  const buildThumbnailUrl = useCallback(
    (path: string, size = 160) => `/api/thumb${encodePath(path)}?auth=${downloadQuery}&size=${size}`,
    [downloadQuery],
  )
  const buildThumbnailFailureKey = useCallback((path: string, size: number) => `${path}::${size}`, [])
  const canUseThumbnail = useCallback(
    (entry: FileEntry, size = 160) =>
      filesScope !== 'trash' &&
      !entry.is_dir &&
      getFilePreviewKind(entry) === 'image' &&
      !thumbLoadFailed[buildThumbnailFailureKey(entry.path, size)],
    [buildThumbnailFailureKey, filesScope, thumbLoadFailed],
  )
  const publicPathSegments = useMemo(() => publicSharePath.split('/').filter(Boolean), [publicSharePath])
  const currentPathSegments = useMemo(() => currentPath.split('/').filter(Boolean), [currentPath])
  const activeUploadCount = useMemo(
    () => uploadProgress.filter((item) => item.status === 'uploading' || item.status === 'paused').length,
    [uploadProgress],
  )
  const rowActionItemClass =
    'flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-semibold text-slate-700 transition hover:bg-slate-50 hover:text-primary'
  const compactToolbarButtonClass =
    'inline-flex size-8 items-center justify-center rounded-lg text-slate-700 transition hover:bg-slate-100 hover:text-slate-950 disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent disabled:hover:text-slate-700'
  const compactPrimaryToolbarButtonClass =
    'inline-flex size-8 items-center justify-center rounded-lg text-slate-800 transition hover:bg-slate-100 hover:text-slate-950 disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-transparent'
  const compactSecondaryToolbarButtonClass =
    'inline-flex size-8 items-center justify-center rounded-lg text-slate-700 transition hover:bg-slate-100 hover:text-slate-950 disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent disabled:hover:text-slate-700'
  const openActionEntry = useMemo(
    () => visibleItems.find((entry) => entry.path === openActionPath) ?? null,
    [visibleItems, openActionPath],
  )
  const favoritePathSet = useMemo(() => new Set(favoriteItems.map((item) => item.path)), [favoriteItems])
  const openActionTrashItem = useMemo(
    () => trashItems.find((item) => item.path === openActionPath) ?? null,
    [openActionPath, trashItems],
  )

  const openActionMenuAt = useCallback((entryPath: string, clientX: number, clientY: number) => {
    if (typeof window === 'undefined') {
      return
    }

    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight

    let left = clientX
    left = Math.max(
      ROW_ACTION_MENU_VIEWPORT_PADDING,
      Math.min(left, viewportWidth - ROW_ACTION_MENU_WIDTH - ROW_ACTION_MENU_VIEWPORT_PADDING),
    )

    let top = clientY + ROW_ACTION_MENU_GAP
    if (top + ROW_ACTION_MENU_ESTIMATED_HEIGHT > viewportHeight - ROW_ACTION_MENU_VIEWPORT_PADDING) {
      top = Math.max(
        ROW_ACTION_MENU_VIEWPORT_PADDING,
        clientY - ROW_ACTION_MENU_ESTIMATED_HEIGHT - ROW_ACTION_MENU_GAP,
      )
    }

    setActionMenuPosition({ top, left })
    setOpenActionPath(entryPath)
  }, [])

  const ensureEntrySelectedForActionMenu = useCallback((entryPath: string) => {
    setSelectedPaths((prev) => (prev.includes(entryPath) ? prev : [entryPath]))
  }, [])

  const openEntryActionMenu = useCallback(
    (entry: FileEntry, clientX: number, clientY: number) => {
      ensureEntrySelectedForActionMenu(entry.path)
      openActionMenuAt(entry.path, clientX, clientY)
    },
    [ensureEntrySelectedForActionMenu, openActionMenuAt],
  )

  const clearTouchLongPress = useCallback(() => {
    if (touchLongPressTimerRef.current != null) {
      window.clearTimeout(touchLongPressTimerRef.current)
      touchLongPressTimerRef.current = null
    }
    touchLongPressStateRef.current = null
  }, [])

  const onEntryContextMenu = useCallback(
    (event: ReactMouseEvent<HTMLElement>, entry: FileEntry) => {
      event.preventDefault()
      event.stopPropagation()
      openEntryActionMenu(entry, event.clientX, event.clientY)
    },
    [openEntryActionMenu],
  )

  const onEntryPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLElement>, entry: FileEntry) => {
      if (event.pointerType !== 'touch') {
        return
      }
      const target = event.target as HTMLElement | null
      if (target?.closest('input[type="checkbox"]')) {
        return
      }

      clearTouchLongPress()
      touchLongPressStateRef.current = {
        pointerId: event.pointerId,
        entryPath: entry.path,
        startX: event.clientX,
        startY: event.clientY,
      }

      touchLongPressTimerRef.current = window.setTimeout(() => {
        const state = touchLongPressStateRef.current
        if (!state || state.entryPath !== entry.path) {
          return
        }

        suppressClickUntilRef.current = Date.now() + LONG_PRESS_CLICK_SUPPRESS_MS
        openEntryActionMenu(entry, state.startX, state.startY)
        clearTouchLongPress()
      }, TOUCH_LONG_PRESS_DELAY_MS)
    },
    [clearTouchLongPress, openEntryActionMenu],
  )

  const onEntryPointerMove = useCallback(
    (event: ReactPointerEvent<HTMLElement>) => {
      const state = touchLongPressStateRef.current
      if (!state || state.pointerId !== event.pointerId) {
        return
      }

      if (
        Math.abs(event.clientX - state.startX) > TOUCH_LONG_PRESS_MOVE_TOLERANCE ||
        Math.abs(event.clientY - state.startY) > TOUCH_LONG_PRESS_MOVE_TOLERANCE
      ) {
        clearTouchLongPress()
      }
    },
    [clearTouchLongPress],
  )

  const onEntryPointerEnd = useCallback(
    (event: ReactPointerEvent<HTMLElement>) => {
      const state = touchLongPressStateRef.current
      if (!state || state.pointerId !== event.pointerId) {
        return
      }
      clearTouchLongPress()
    },
    [clearTouchLongPress],
  )

  const onEntryClickCapture = useCallback((event: ReactMouseEvent<HTMLElement>) => {
    if (Date.now() <= suppressClickUntilRef.current) {
      event.preventDefault()
      event.stopPropagation()
    }
  }, [])

  useEffect(() => {
    if (!openActionPath || !actionMenuPosition || typeof window === 'undefined') {
      return
    }
    const menuElement = rowActionMenuRef.current
    if (!menuElement) {
      return
    }

    const rect = menuElement.getBoundingClientRect()
    const maxLeft = Math.max(ROW_ACTION_MENU_VIEWPORT_PADDING, window.innerWidth - rect.width - ROW_ACTION_MENU_VIEWPORT_PADDING)
    const maxTop = Math.max(ROW_ACTION_MENU_VIEWPORT_PADDING, window.innerHeight - rect.height - ROW_ACTION_MENU_VIEWPORT_PADDING)
    const nextLeft = Math.max(ROW_ACTION_MENU_VIEWPORT_PADDING, Math.min(actionMenuPosition.left, maxLeft))
    const nextTop = Math.max(ROW_ACTION_MENU_VIEWPORT_PADDING, Math.min(actionMenuPosition.top, maxTop))

    if (nextLeft !== actionMenuPosition.left || nextTop !== actionMenuPosition.top) {
      setActionMenuPosition({ top: nextTop, left: nextLeft })
    }
  }, [openActionPath, actionMenuPosition])

  useEffect(() => () => {
    clearTouchLongPress()
  }, [clearTouchLongPress])

  useEffect(() => {
    setThumbLoadFailed({})
  }, [visibleItems])

  useEffect(() => {
    if (!renameEditingExtension) {
      return
    }
    renameExtensionInputRef.current?.focus()
    renameExtensionInputRef.current?.select()
  }, [renameEditingExtension])

  useEffect(() => {
    if (!renameTarget) {
      return
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') {
        return
      }
      event.preventDefault()
      closeRenameModal()
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [closeRenameModal, renameTarget])

  const uploadQueueCount = uploadProgress.filter((item) => item.status !== 'completed').length
  const previewIsDocFile = useMemo(() => {
    if (!previewEntry || previewKind !== 'office') {
      return false
    }
    return isDocFileName(previewEntry.name)
  }, [previewEntry, previewKind])
  const previewRawUrl = useMemo(() => {
    if (!previewEntry) {
      return ''
    }
    return buildRawFileUrl(previewEntry.path)
  }, [buildRawFileUrl, previewEntry])
  const previewDocPdfUrl = useMemo(() => {
    if (!previewEntry || !previewIsDocFile) {
      return ''
    }
    return `/api/preview/pdf${encodePath(previewEntry.path)}?auth=${downloadQuery}`
  }, [downloadQuery, previewEntry, previewIsDocFile])
  const publicImageRawUrl = useMemo(() => {
    if (!publicShareId) {
      return ''
    }
    return buildPublicInlinePathForTarget(publicShareId, publicSharePath, publicSharePassword)
  }, [publicShareId, publicSharePassword, publicSharePath])
  const officePreviewUrl = useMemo(() => {
    if (!previewEntry || previewKind !== 'office' || !previewRawUrl || previewIsDocFile) {
      return ''
    }
    return `https://view.officeapps.live.com/op/embed.aspx?src=${encodeURIComponent(`${window.location.origin}${previewRawUrl}`)}`
  }, [previewEntry, previewKind, previewRawUrl, previewIsDocFile])
  const displayPreviewKind = useMemo<FilePreviewKind>(() => {
    if (previewIsDocFile && previewDocPdfSrc) {
      return 'pdf'
    }
    return previewKind
  }, [previewDocPdfSrc, previewIsDocFile, previewKind])
  const displayPreviewRawUrl = useMemo(() => {
    if (previewIsDocFile) {
      return previewDocPdfSrc
    }
    return previewRawUrl
  }, [previewDocPdfSrc, previewIsDocFile, previewRawUrl])
  const publicTextContent = useMemo(() => {
    if (!publicShareData || publicShareData.is_dir || publicShareData.content == null) {
      return ''
    }
    return publicShareData.content
  }, [publicShareData])
  const publicSharePreviewKind = useMemo(() => {
    if (!publicShareData || publicShareData.is_dir) {
      return 'unknown' as FilePreviewKind
    }
    return getFilePreviewKind({ name: fileNameFromPath(publicSharePath) })
  }, [publicShareData, publicSharePath])
  const publicShareIsImage = publicSharePreviewKind === 'image'
  const publicInlineRawUrl = useMemo(() => {
    if (!publicShareId || !publicShareData || publicShareData.is_dir) {
      return ''
    }
    return buildPublicInlinePathForTarget(publicShareId, publicSharePath, publicSharePassword)
  }, [publicShareData, publicShareId, publicSharePassword, publicSharePath])
  const publicOfficePreviewUrl = useMemo(() => {
    if (
      !publicInlineRawUrl ||
      (publicSharePreviewKind !== 'office' && publicSharePreviewKind !== 'docx')
    ) {
      return ''
    }
    return `https://view.officeapps.live.com/op/embed.aspx?src=${encodeURIComponent(`${window.location.origin}${publicInlineRawUrl}`)}`
  }, [publicInlineRawUrl, publicSharePreviewKind])
  const publicPreviewIsTruncated = publicTextContent.length > PUBLIC_TEXT_PREVIEW_LIMIT
  const publicPreviewContent =
    publicPreviewIsTruncated && !publicPreviewExpanded
      ? `${publicTextContent.slice(0, PUBLIC_TEXT_PREVIEW_LIMIT)}\n\n... (preview truncated)`
      : publicTextContent
  const publicTextMode = useMemo(() => {
    if (!publicShareData || publicShareData.is_dir || publicShareData.content == null) {
      return 'plain'
    }
    return getTextPreviewMode(fileNameFromPath(publicSharePath))
  }, [publicShareData, publicSharePath])
  const publicMarkdownHtml = useMemo(() => {
    if (publicTextMode !== 'markdown') {
      return ''
    }
    return renderMarkdownHtml(publicPreviewContent)
  }, [publicPreviewContent, publicTextMode])
  const currentUserName = useMemo(() => {
    if (!effectiveToken) {
      return ''
    }
    try {
      const payload = effectiveToken.split('.')[1]
      if (!payload) {
        return ''
      }
      const normalized = payload.replace(/-/g, '+').replace(/_/g, '/')
      const decoded = JSON.parse(atob(normalized)) as { sub?: unknown }
      return typeof decoded.sub === 'string' ? decoded.sub : ''
    } catch {
      return ''
    }
  }, [effectiveToken])
  const visibleFolderCount = useMemo(() => visibleItems.filter((item) => item.is_dir).length, [visibleItems])
  const visibleFileCount = useMemo(() => visibleItems.filter((item) => !item.is_dir).length, [visibleItems])

  useEffect(() => {
    let cancelled = false
    const controller = new AbortController()
    let tickTimer: number | null = null

    setPreviewDocPdfSrc((prev) => {
      revokeBlobUrl(prev)
      return ''
    })

    if (!previewEntry || !previewIsDocFile || !previewDocPdfUrl) {
      return () => {
        cancelled = true
        controller.abort()
        if (tickTimer != null) {
          window.clearInterval(tickTimer)
        }
      }
    }

    const startedAt = Date.now()
    setPreviewError('')
    setPreviewLoading(true)
    setPreviewLoadingLabel('Converting document to PDF...')
    setPreviewLoadingElapsedSeconds(0)
    tickTimer = window.setInterval(() => {
      const elapsedSeconds = Math.max(1, Math.floor((Date.now() - startedAt) / 1000))
      setPreviewLoadingElapsedSeconds(elapsedSeconds)
      if (elapsedSeconds >= 3) {
        setPreviewLoadingLabel('Still converting, this may take a while...')
      }
    }, 1000)

    void fetch(previewDocPdfUrl, {
      signal: controller.signal,
      headers: withAuthHeaders(effectiveToken),
    })
      .then(async (response) => {
        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          throw new Error(payload.error ?? `Document preview failed (${response.status})`)
        }
        return response.blob()
      })
      .then((blob) => {
        if (cancelled) {
          return
        }
        const objectUrl = URL.createObjectURL(blob)
        setPreviewDocPdfSrc((prev) => {
          revokeBlobUrl(prev)
          return objectUrl
        })
      })
      .catch((error) => {
        if (cancelled || controller.signal.aborted) {
          return
        }
        console.error('doc preview conversion failed', error)
        const message = error instanceof Error ? error.message : String(error)
        setPreviewError(message || 'Document conversion failed. Please download and open locally.')
      })
      .finally(() => {
        if (cancelled) {
          return
        }
        setPreviewLoading(false)
        if (tickTimer != null) {
          window.clearInterval(tickTimer)
        }
      })

    return () => {
      cancelled = true
      controller.abort()
      if (tickTimer != null) {
        window.clearInterval(tickTimer)
      }
    }
  }, [effectiveToken, previewDocPdfUrl, previewEntry, previewIsDocFile])

  useEffect(() => {
    let cancelled = false
    const controller = new AbortController()
    let tickTimer: number | null = null

    setPreviewDocxHtml('')

    if (!previewEntry || previewKind !== 'docx' || !previewRawUrl) {
      return () => {
        cancelled = true
        controller.abort()
        if (tickTimer != null) {
          window.clearInterval(tickTimer)
        }
      }
    }

    const startedAt = Date.now()
    setPreviewError('')
    setPreviewLoading(true)
    setPreviewLoadingLabel('Parsing DOCX preview...')
    setPreviewLoadingElapsedSeconds(0)
    tickTimer = window.setInterval(() => {
      const elapsedSeconds = Math.max(1, Math.floor((Date.now() - startedAt) / 1000))
      setPreviewLoadingElapsedSeconds(elapsedSeconds)
      if (elapsedSeconds >= 3) {
        setPreviewLoadingLabel('Still parsing DOCX content...')
      }
    }, 1000)

    void fetch(previewRawUrl, {
      signal: controller.signal,
      headers: withAuthHeaders(effectiveToken),
    })
      .then(async (response) => {
        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          throw new Error(payload.error ?? `DOCX preview failed (${response.status})`)
        }
        return response.arrayBuffer()
      })
      .then(async (buffer) => {
        const result = await mammoth.convertToHtml({ arrayBuffer: buffer })
        if (cancelled) {
          return
        }
        setPreviewDocxHtml(result.value || '')
        if (!result.value?.trim()) {
          setPreviewError('This DOCX file has no previewable content.')
        }
      })
      .catch((error) => {
        if (cancelled || controller.signal.aborted) {
          return
        }
        console.error('docx preview render failed', error)
        const message = error instanceof Error ? error.message : String(error)
        setPreviewError(message || 'DOCX preview failed. Please download and open locally.')
      })
      .finally(() => {
        if (cancelled) {
          return
        }
        setPreviewLoading(false)
        if (tickTimer != null) {
          window.clearInterval(tickTimer)
        }
      })

    return () => {
      cancelled = true
      controller.abort()
      if (tickTimer != null) {
        window.clearInterval(tickTimer)
      }
    }
  }, [effectiveToken, previewEntry, previewKind, previewRawUrl])

  useEffect(() => {
    let cancelled = false
    const controller = new AbortController()

    setPreviewImageSrc((prev) => {
      revokeBlobUrl(prev)
      return ''
    })
    setPreviewImageLoadedBytes(0)
    setPreviewImageTotalBytes(null)
    setPreviewImageProgressPercent(null)

    if (!previewEntry || previewKind !== 'image' || !previewRawUrl) {
      setPreviewImageLoading(false)
      return () => {
        cancelled = true
        controller.abort()
      }
    }

    setPreviewError('')
    setPreviewImageLoading(true)
    void downloadImageWithProgress(previewRawUrl, controller.signal, (progress) => {
      if (cancelled) {
        return
      }
      setPreviewImageLoadedBytes(progress.loadedBytes)
      setPreviewImageTotalBytes(progress.totalBytes)
      setPreviewImageProgressPercent(progress.progressPercent)
    })
      .then((blob) => {
        if (cancelled) {
          return
        }
        const objectUrl = URL.createObjectURL(blob)
        setPreviewImageSrc((prev) => {
          revokeBlobUrl(prev)
          return objectUrl
        })
      })
      .catch((err) => {
        if (cancelled || controller.signal.aborted) {
          return
        }
        console.error('image preview download failed', err)
        setPreviewImageLoading(false)
        setPreviewError('Image preview failed to download. Please retry.')
      })

    return () => {
      cancelled = true
      controller.abort()
    }
  }, [previewEntry, previewKind, previewRawUrl])

  useEffect(() => {
    let cancelled = false
    const controller = new AbortController()

    setPublicImageDisplaySrc((prev) => {
      revokeBlobUrl(prev)
      return ''
    })
    setPublicImageLoadedBytes(0)
    setPublicImageTotalBytes(null)
    setPublicImageProgressPercent(null)

    if (!publicShareData || publicShareData.is_dir || !publicShareIsImage || !publicImageRawUrl) {
      setPublicImageLoading(false)
      return () => {
        cancelled = true
        controller.abort()
      }
    }

    setPublicShareError('')
    setPublicImageLoading(true)
    void downloadImageWithProgress(publicImageRawUrl, controller.signal, (progress) => {
      if (cancelled) {
        return
      }
      setPublicImageLoadedBytes(progress.loadedBytes)
      setPublicImageTotalBytes(progress.totalBytes)
      setPublicImageProgressPercent(progress.progressPercent)
    })
      .then((blob) => {
        if (cancelled) {
          return
        }
        const objectUrl = URL.createObjectURL(blob)
        setPublicImageDisplaySrc((prev) => {
          revokeBlobUrl(prev)
          return objectUrl
        })
      })
      .catch((err) => {
        if (cancelled || controller.signal.aborted) {
          return
        }
        console.error('public image preview download failed', err)
        setPublicImageLoading(false)
        setPublicShareError('Image preview failed to download. Please retry.')
      })

    return () => {
      cancelled = true
      controller.abort()
    }
  }, [publicImageRawUrl, publicShareData, publicShareIsImage])

  useEffect(() => {
    return () => {
      revokeBlobUrl(previewImageSrc)
      revokeBlobUrl(previewDocPdfSrc)
      revokeBlobUrl(publicImageDisplaySrc)
    }
  }, [previewDocPdfSrc, previewImageSrc, publicImageDisplaySrc])

  const deleteToastNode = deleteToast ? (
    <div className="pointer-events-none fixed inset-0 z-[80] flex items-center justify-center px-4">
      <div className="max-w-[min(92vw,420px)] rounded-xl border border-emerald-200 bg-emerald-50/95 px-3 py-2 text-sm font-semibold text-emerald-800 shadow-lg shadow-emerald-900/10 backdrop-blur-sm">
        {deleteToast}
      </div>
    </div>
  ) : null

  const renameModalNode = renameTarget ? (
    <div
      className="fixed inset-0 z-[85] flex items-center justify-center bg-slate-900/45 px-4 py-6 backdrop-blur-sm"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          closeRenameModal()
        }
      }}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault()
          void onSubmitRename()
        }}
        className="w-full max-w-lg rounded-3xl border border-slate-200 bg-white p-5 shadow-2xl"
      >
        <div className="mb-4">
          <h3 className="text-lg font-semibold text-slate-900">Rename {renameTarget.is_dir ? 'Folder' : 'File'}</h3>
          <p className="mt-1 text-xs text-slate-500">
            <span className="font-medium text-slate-600">Current name:</span>{' '}
            <span className="break-all">{renameTarget.name}</span>
          </p>
        </div>

        <label className="block text-sm font-semibold text-slate-700" htmlFor="rename-base-input">
          Name
        </label>
        <div className="mt-2 flex items-start gap-2">
          <input
            id="rename-base-input"
            value={renameBaseName}
            onChange={(event) => {
              setRenameBaseName(event.target.value)
              if (renameError) {
                setRenameError('')
              }
            }}
            autoFocus
            disabled={renameSubmitting}
            className="min-w-0 flex-1 rounded-xl border border-slate-300 px-3 py-2 text-sm text-slate-800 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20 disabled:cursor-not-allowed disabled:bg-slate-100"
            placeholder={renameTarget.is_dir ? 'Folder name' : 'File name'}
          />
          {!renameTarget.is_dir ? (
            <div className="w-[168px] shrink-0">
              {renameEditingExtension ? (
                <input
                  ref={renameExtensionInputRef}
                  value={renameExtension}
                  onChange={(event) => {
                    setRenameExtension(event.target.value)
                    if (renameError) {
                      setRenameError('')
                    }
                  }}
                  onBlur={() => setRenameEditingExtension(false)}
                  disabled={renameSubmitting}
                  className="w-full rounded-xl border border-slate-300 px-3 py-2 text-sm text-slate-800 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20 disabled:cursor-not-allowed disabled:bg-slate-100"
                  placeholder="ext"
                />
              ) : (
                <button
                  type="button"
                  onClick={() => setRenameEditingExtension(true)}
                  disabled={renameSubmitting}
                  className="w-full rounded-xl border border-slate-300 bg-slate-50 px-3 py-2 text-left text-sm font-semibold text-slate-700 transition hover:border-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-60"
                >
                  {renameExtension.trim() ? `.${renameExtension.trim().replace(/^\.+/, '')}` : '.(none)'}
                </button>
              )}
              <button
                type="button"
                onClick={() => setRenameEditingExtension((prev) => !prev)}
                disabled={renameSubmitting}
                className="mt-1 text-[11px] font-medium text-primary transition hover:text-teal-700 disabled:cursor-not-allowed disabled:opacity-60"
              >
                {renameEditingExtension ? 'Finish extension edit' : 'Click to edit extension'}
              </button>
            </div>
          ) : null}
        </div>

        {renameError ? <p className="mt-2 text-sm text-rose-600">{renameError}</p> : null}

        <div className="mt-5 flex items-center justify-end gap-2">
          <button
            type="button"
            onClick={() => closeRenameModal()}
            disabled={renameSubmitting}
            className="rounded-xl border border-slate-300 px-4 py-2 text-sm font-semibold text-slate-700 transition hover:border-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-60"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={renameSubmitting}
            className="rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-white transition hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-60"
          >
            {renameSubmitting ? 'Renaming...' : 'Rename'}
          </button>
        </div>
      </form>
    </div>
  ) : null

  const confirmDialogNode = confirmDialog ? (
    <ActionDialog
      open
      title={confirmDialog.title}
      description={confirmDialog.description}
      confirmLabel={confirmDialogBusy ? 'Working...' : confirmDialog.confirmLabel}
      confirmTone={confirmDialog.confirmTone}
      busy={confirmDialogBusy}
      onCancel={closeConfirmDialog}
      onConfirm={() => {
        void submitConfirmDialog()
      }}
    />
  ) : null

  const folderDialogNode = (
    <ActionDialog
      open={folderDialogOpen}
      title="Create folder"
      description={<p>Create a new folder under <span className="font-semibold text-slate-800">{currentPath}</span>.</p>}
      confirmLabel={folderDialogBusy ? 'Creating...' : 'Create'}
      confirmDisabled={!folderNameInput.trim()}
      busy={folderDialogBusy}
      onCancel={closeFolderDialog}
      onSubmit={(event) => {
        event.preventDefault()
        void submitCreateFolderDialog()
      }}
    >
      <label className="block text-sm font-semibold text-slate-700" htmlFor="create-folder-name">
        Folder name
      </label>
      <input
        id="create-folder-name"
        value={folderNameInput}
        onChange={(event) => {
          setFolderNameInput(event.target.value)
          if (folderDialogError) {
            setFolderDialogError('')
          }
        }}
        autoFocus
        disabled={folderDialogBusy}
        className="mt-1 w-full rounded-xl border border-slate-300 px-3 py-2 text-sm text-slate-800 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20 disabled:cursor-not-allowed disabled:bg-slate-100"
        placeholder="Folder name"
      />
      {folderDialogError ? <p className="text-sm text-rose-600">{folderDialogError}</p> : null}
    </ActionDialog>
  )

  const shareDialogNode = shareDialog ? (
    <ActionDialog
      open
      title="Create share link"
      description={
        <p>
          Create link for <span className="font-semibold text-slate-800">{shareDialog.entry.path}</span>.
        </p>
      }
      confirmLabel={shareDialogBusy ? 'Creating...' : 'Create share'}
      busy={shareDialogBusy}
      onCancel={closeShareDialog}
      onSubmit={(event) => {
        event.preventDefault()
        void submitShareDialog()
      }}
    >
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="block text-sm font-semibold text-slate-700" htmlFor="share-expire-input">
          Expiration (seconds)
          <input
            id="share-expire-input"
            value={shareDialog.expiresInSeconds}
            onChange={(event) => {
              const nextValue = event.target.value
              setShareDialog((prev) => (prev ? { ...prev, expiresInSeconds: nextValue, error: '' } : prev))
            }}
            autoFocus
            disabled={shareDialogBusy}
            className="mt-1 w-full rounded-xl border border-slate-300 px-3 py-2 text-sm text-slate-800 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20 disabled:cursor-not-allowed disabled:bg-slate-100"
            placeholder="empty for no expiration"
            inputMode="numeric"
          />
        </label>
        <label className="block text-sm font-semibold text-slate-700" htmlFor="share-password-input">
          Password (optional)
          <input
            id="share-password-input"
            type="password"
            value={shareDialog.password}
            onChange={(event) => {
              const nextValue = event.target.value
              setShareDialog((prev) => (prev ? { ...prev, password: nextValue, error: '' } : prev))
            }}
            disabled={shareDialogBusy}
            className="mt-1 w-full rounded-xl border border-slate-300 px-3 py-2 text-sm text-slate-800 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20 disabled:cursor-not-allowed disabled:bg-slate-100"
            placeholder="No password"
          />
        </label>
      </div>
      {shareDialog.error ? <p className="text-sm text-rose-600">{shareDialog.error}</p> : null}
    </ActionDialog>
  ) : null

  const moveCopyDialogNode = moveCopyDialog ? (
    <ActionDialog
      open
      title={moveCopyDialog.mode === 'single'
        ? `${moveCopyDialog.action === 'move' ? 'Move' : 'Copy'} item`
        : `${moveCopyDialog.action === 'move' ? 'Move' : 'Copy'} selected items`}
      description={
        moveCopyDialog.mode === 'single' && moveCopyDialog.sourceEntry ? (
          <p>
            {`${moveCopyDialog.action === 'move' ? 'Move' : 'Copy'} `}
            <span className="font-semibold text-slate-800">{moveCopyDialog.sourceEntry.path}</span>
          </p>
        ) : (
          <p>{`${moveCopyDialog.action === 'move' ? 'Move' : 'Copy'} ${selectedEntries.length} selected item(s).`}</p>
        )
      }
      confirmLabel={moveCopyDialogBusy ? 'Applying...' : moveCopyDialog.action === 'move' ? 'Move' : 'Copy'}
      busy={moveCopyDialogBusy}
      onCancel={closeMoveCopyDialog}
      onSubmit={(event) => {
        event.preventDefault()
        void submitMoveCopyDialog()
      }}
    >
      <label className="block text-sm font-semibold text-slate-700" htmlFor="move-copy-destination-input">
        Destination {moveCopyDialog.mode === 'single' ? 'path' : 'directory'}
        <input
          id="move-copy-destination-input"
          value={moveCopyDialog.destination}
          onChange={(event) => {
            const nextValue = event.target.value
            setMoveCopyDialog((prev) => (prev ? { ...prev, destination: nextValue, error: '' } : prev))
          }}
          autoFocus
          disabled={moveCopyDialogBusy}
          className="mt-1 w-full rounded-xl border border-slate-300 px-3 py-2 text-sm text-slate-800 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20 disabled:cursor-not-allowed disabled:bg-slate-100"
          placeholder={moveCopyDialog.mode === 'single' ? '/target/path' : '/target/directory'}
        />
      </label>
      <label className="flex items-center gap-2 text-sm font-medium text-slate-700">
        <input
          type="checkbox"
          checked={moveCopyDialog.overrideExisting}
          onChange={(event) => {
            const checked = event.target.checked
            setMoveCopyDialog((prev) => (prev ? { ...prev, overrideExisting: checked, error: '' } : prev))
          }}
          disabled={moveCopyDialogBusy}
          className="size-4 rounded border-slate-300 text-primary focus:ring-primary/30"
        />
        Override existing targets if conflicts occur
      </label>
      {moveCopyDialog.error ? <p className="text-sm text-rose-600">{moveCopyDialog.error}</p> : null}
    </ActionDialog>
  ) : null

  if (publicShareId) {
    return (
      <main className="bucky-file-app min-h-screen bg-[radial-gradient(circle_at_top,#d7ece8,transparent_55%),#f4f8f7] px-4 py-6 md:px-8">
        <section className="mx-auto w-full max-w-4xl rounded-3xl border border-slate-200 bg-white shadow-sm">
          <header className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-5 py-4">
            <div>
              <h1 className="text-xl font-semibold text-slate-900">Shared with you</h1>
              <p className="text-sm text-slate-600">Share ID: {publicShareId}</p>
            </div>
              <a
                href="/"
                className="rounded-lg border border-slate-300 px-3 py-2 text-sm font-medium text-slate-700 transition hover:border-primary hover:text-primary"
              >
                Open file manager
              </a>
          </header>

          <div className="border-b border-slate-200 px-5 py-4">
            <div className="flex flex-wrap items-end gap-3">
              <label className="block text-sm text-slate-700">
                Share password (optional)
                <input
                  type="password"
                  value={publicSharePassword}
                  onChange={(event) => setPublicSharePassword(event.target.value)}
                  className="mt-1 min-w-[240px] rounded-xl border border-slate-300 px-3 py-2 text-sm outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20"
                  placeholder="Enter password if required"
                />
              </label>
              <button
                type="button"
                onClick={() => void loadPublicShare(publicShareId, publicSharePassword, publicSharePath)}
                disabled={publicShareLoading}
                className="rounded-lg bg-primary px-4 py-2 text-sm font-semibold text-white transition hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-60"
              >
                <span className="inline-flex items-center gap-1.5">
                  <Icon name="open" />
                  {publicShareLoading ? 'Loading...' : 'Open share'}
                </span>
              </button>
            </div>
            {publicShareError ? <p className="mt-3 text-sm text-rose-600">{publicShareError}</p> : null}
          </div>

          {publicShareData ? (
            <section className="px-5 py-4">
              <div className="mb-3 flex flex-wrap items-center gap-2">
                <p className="text-sm font-semibold text-slate-800">{publicShareData.share.path}</p>
                <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] font-semibold text-slate-600">
                  {publicShareData.is_dir ? 'Folder' : 'File'}
                </span>
                <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] font-semibold text-slate-600">
                  {publicShareData.share.password_required ? 'Password protected' : 'Public'}
                </span>
                <span className="rounded-full bg-slate-100 px-2 py-0.5 text-[11px] font-semibold text-slate-600">
                  {publicShareData.share.expires_at
                    ? `Expires ${formatTimestamp(publicShareData.share.expires_at)}`
                    : 'No expiration'}
                </span>
              </div>

              <div className="mb-3 flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  onClick={() => void loadPublicShare(publicShareId, publicSharePassword, '/')}
                  className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="folder" />
                    Root
                  </span>
                </button>
                {publicPathSegments.map((segment, index) => {
                  const partialPath = `/${publicPathSegments.slice(0, index + 1).join('/')}`
                  return (
                    <button
                      key={partialPath}
                      type="button"
                      onClick={() => void loadPublicShare(publicShareId, publicSharePassword, partialPath)}
                      className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="folder" />
                        {segment}
                      </span>
                    </button>
                  )
                })}
                {publicShareData.parent_path && publicShareData.parent_path !== publicSharePath ? (
                  <button
                    type="button"
                    onClick={() => void loadPublicShare(publicShareId, publicSharePassword, publicShareData.parent_path || '/')}
                    className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                  >
                    <span className="inline-flex items-center gap-1.5">
                      <Icon name="up" />
                      Up
                    </span>
                  </button>
                ) : null}
                <a
                  href={buildPublicDownloadPathForTarget(publicShareId, publicSharePath, publicSharePassword)}
                  className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="download" />
                    Download current
                  </span>
                </a>
              </div>

              {publicShareData.is_dir ? (
                <div className="overflow-x-auto rounded-xl border border-slate-200">
                  <table className="w-full min-w-[640px] text-sm">
                    <thead className="bg-slate-50 text-left text-xs uppercase tracking-wide text-slate-500">
                      <tr>
                        <th className="px-3 py-2">Name</th>
                        <th className="px-3 py-2">Type</th>
                        <th className="px-3 py-2">Size</th>
                        <th className="px-3 py-2">Modified</th>
                        <th className="px-3 py-2">Actions</th>
                      </tr>
                    </thead>
                    <tbody>
                      {(publicShareData.items ?? []).length === 0 ? (
                        <tr>
                          <td colSpan={5} className="px-3 py-8 text-center text-slate-500">
                            This shared folder is empty.
                          </td>
                        </tr>
                      ) : (
                        (publicShareData.items ?? []).map((item) => {
                          const itemIcon = getEntryIconMeta(item)
                          return (
                          <tr key={item.path} className="border-t border-slate-100 text-slate-800">
                            <td className="px-3 py-2 font-medium">
                              {item.is_dir ? (
                                <button
                                  type="button"
                                  onClick={() => void loadPublicShare(publicShareId, publicSharePassword, item.path)}
                                  className="rounded px-1 py-0.5 text-left text-primary transition hover:bg-primary/10"
                                  aria-label={item.name}
                                >
                                  <span className="inline-flex items-center gap-1.5">
                                    <Icon name="folder" className="shrink-0" />
                                    <FileNameTooltip name={item.name} maxChars={38} maxWidthClass="max-w-[360px]" />
                                  </span>
                                </button>
                              ) : (
                                <span className="inline-flex items-center gap-1.5" aria-label={item.name}>
                                  <Icon name={itemIcon.iconName} className={`${itemIcon.iconClassName ?? ''} shrink-0`.trim()} />
                                  <FileNameTooltip name={item.name} maxChars={38} maxWidthClass="max-w-[360px]" />
                                </span>
                              )}
                            </td>
                            <td className="px-3 py-2">{item.is_dir ? 'Folder' : 'File'}</td>
                            <td className="px-3 py-2">{item.is_dir ? '-' : formatBytes(item.size)}</td>
                            <td className="px-3 py-2">{formatTimestamp(item.modified)}</td>
                            <td className="px-3 py-2">
                              <div className="flex items-center gap-2">
                                {!item.is_dir ? (
                                  <button
                                    type="button"
                                    onClick={() => void loadPublicShare(publicShareId, publicSharePassword, item.path)}
                                    className="rounded border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                                  >
                                    <span className="inline-flex items-center gap-1.5">
                                      <Icon name="open" />
                                      Preview
                                    </span>
                                  </button>
                                ) : null}
                                <a
                                  href={buildPublicDownloadPathForTarget(publicShareId, item.path, publicSharePassword)}
                                  className="rounded border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                                >
                                  <span className="inline-flex items-center gap-1.5">
                                    <Icon name="download" />
                                    Download
                                  </span>
                                </a>
                              </div>
                            </td>
                          </tr>
                          )
                        })
                      )}
                    </tbody>
                  </table>
                </div>
              ) : (
                <div className="space-y-3">
                  <div className="flex flex-wrap items-center gap-3 text-sm text-slate-600">
                    <span>Size: {formatBytes(publicShareData.size ?? 0)}</span>
                    <span>Modified: {formatTimestamp(publicShareData.modified ?? 0)}</span>
                    <a
                      href={buildPublicDownloadPathForTarget(publicShareId, publicSharePath, publicSharePassword)}
                      className="rounded-lg border border-slate-300 px-3 py-2 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="download" />
                        Download file
                      </span>
                    </a>
                    {publicShareData.parent_path ? (
                      <button
                        type="button"
                        onClick={() => void loadPublicShare(publicShareId, publicSharePassword, publicShareData.parent_path || '/')}
                        className="rounded-lg border border-slate-300 px-3 py-2 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                      >
                        <span className="inline-flex items-center gap-1.5">
                          <Icon name="folder" />
                          Back to folder
                        </span>
                      </button>
                    ) : null}
                  </div>

                  {publicShareData.content != null ? (
                    <>
                      {publicTextMode === 'markdown' && publicMarkdownHtml.trim() ? (
                        <article
                          className="max-h-[560px] overflow-auto rounded-xl border border-slate-200 bg-white p-4 text-sm leading-7 text-slate-800 [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_blockquote]:my-3 [&_blockquote]:rounded-r-lg [&_blockquote]:border-l-4 [&_blockquote]:border-primary/40 [&_blockquote]:bg-teal-50/40 [&_blockquote]:px-3 [&_blockquote]:py-2 [&_blockquote_code]:rounded [&_blockquote_code]:bg-slate-100 [&_blockquote_code]:px-1 [&_blockquote_code]:py-0.5 [&_blockquote_code]:text-[0.85em] [&_h1]:mt-1 [&_h1]:text-2xl [&_h1]:font-semibold [&_h2]:mt-4 [&_h2]:text-xl [&_h2]:font-semibold [&_h3]:mt-3 [&_h3]:text-lg [&_h3]:font-semibold [&_hr]:my-4 [&_hr]:border-slate-200 [&_li]:my-1 [&_li_code]:rounded [&_li_code]:bg-slate-100 [&_li_code]:px-1 [&_li_code]:py-0.5 [&_li_code]:text-[0.85em] [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_p_code]:rounded [&_p_code]:bg-slate-100 [&_p_code]:px-1 [&_p_code]:py-0.5 [&_p_code]:text-[0.85em] [&_pre]:my-3 [&_pre]:overflow-auto [&_pre]:rounded-lg [&_pre]:bg-slate-950 [&_pre]:p-3 [&_pre]:text-xs [&_pre]:text-slate-100 [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-slate-100 [&_strong]:font-semibold [&_table]:my-3 [&_table]:w-full [&_table]:border-collapse [&_tbody_td]:border [&_tbody_td]:border-slate-200 [&_tbody_td]:px-2 [&_tbody_td]:py-1.5 [&_tbody_tr:nth-child(even)]:bg-slate-50 [&_thead_th]:border [&_thead_th]:border-slate-200 [&_thead_th]:bg-slate-100 [&_thead_th]:px-2 [&_thead_th]:py-1.5 [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5"
                          dangerouslySetInnerHTML={{ __html: publicMarkdownHtml }}
                        />
                      ) : (
                        <pre className="max-h-[420px] overflow-auto rounded-xl border border-slate-200 bg-slate-50 p-3 text-xs text-slate-800">
                          {publicPreviewContent}
                        </pre>
                      )}
                      {publicPreviewIsTruncated ? (
                        <div className="flex items-center justify-between gap-2 text-xs text-slate-600">
                          <span>
                            Preview shows {formatBytes(publicPreviewContent.length)} of {formatBytes(publicTextContent.length)}.
                          </span>
                          <button
                            type="button"
                            onClick={() => setPublicPreviewExpanded((prev) => !prev)}
                            className="rounded-lg border border-slate-300 px-2 py-1 font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                          >
                            <span className="inline-flex items-center gap-1.5">
                              <Icon name="open" />
                              {publicPreviewExpanded ? 'Show truncated preview' : 'Show full text'}
                            </span>
                          </button>
                        </div>
                      ) : null}
                    </>
                  ) : publicShareIsImage ? (
                    <div className="space-y-2 rounded-xl border border-slate-200 bg-slate-50 p-3">
                      <div className="flex justify-end">
                        <button
                          type="button"
                          onClick={() =>
                            openImageViewer(
                              publicImageDisplaySrc || publicImageRawUrl,
                              publicSharePath,
                            )
                          }
                          className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                        >
                          <span className="inline-flex items-center gap-1.5">
                            <Icon name="open" />
                            View original
                          </span>
                        </button>
                      </div>
                      {publicImageDisplaySrc ? (
                        <img
                          src={publicImageDisplaySrc}
                          alt={publicSharePath}
                          className={`mx-auto max-h-[520px] w-auto max-w-full transition-opacity ${publicImageLoading ? 'opacity-0' : 'opacity-100'}`}
                          loading="lazy"
                          onLoad={() => setPublicImageLoading(false)}
                          onError={() => {
                            setPublicImageLoading(false)
                            setPublicShareError('Image preview failed to render. Please reopen the share.')
                          }}
                          onClick={() =>
                            openImageViewer(
                              publicImageDisplaySrc || publicImageRawUrl,
                              publicSharePath,
                            )
                          }
                        />
                      ) : null}
                      {publicImageLoading ? (
                        <div className="flex flex-col items-center gap-2 px-1 py-10 text-center">
                          <ProgressRing progressPercent={publicImageProgressPercent} />
                          <p className="text-xs font-medium text-slate-600">Loading image preview...</p>
                          <p className="text-[11px] text-slate-500">
                            {publicImageTotalBytes != null
                              ? `${formatBytes(publicImageLoadedBytes)} / ${formatBytes(publicImageTotalBytes)}`
                              : formatBytes(publicImageLoadedBytes)}
                          </p>
                        </div>
                      ) : null}
                    </div>
                  ) : publicSharePreviewKind === 'pdf' && publicInlineRawUrl ? (
                    <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
                      <iframe src={publicInlineRawUrl} title={publicSharePath} className="h-[560px] w-full" />
                    </div>
                  ) : publicSharePreviewKind === 'audio' && publicInlineRawUrl ? (
                    <div className="space-y-3 rounded-xl border border-slate-200 bg-white p-3">
                      <audio controls preload="metadata" className="w-full">
                        <source src={publicInlineRawUrl} />
                        Your browser does not support audio preview.
                      </audio>
                      <p className="text-xs text-slate-500">Audio preview</p>
                    </div>
                  ) : publicSharePreviewKind === 'video' && publicInlineRawUrl ? (
                    <div className="space-y-3 rounded-xl border border-slate-200 bg-white p-3">
                      <video controls playsInline preload="metadata" className="max-h-[560px] w-full rounded-lg bg-black" src={publicInlineRawUrl}>
                        Your browser does not support video preview.
                      </video>
                      <p className="text-xs text-slate-500">Video preview</p>
                    </div>
                  ) : publicOfficePreviewUrl ? (
                    <div className="space-y-2">
                      <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
                        <iframe src={publicOfficePreviewUrl} title={`${publicSharePath} (office preview)`} className="h-[560px] w-full" />
                      </div>
                      <p className="text-xs text-slate-500">If office preview fails, download and open locally.</p>
                    </div>
                  ) : (
                    <p className="rounded-xl border border-dashed border-slate-300 bg-slate-50 px-3 py-2 text-sm text-slate-600">
                      Preview is unavailable for this file type. Download to view it locally.
                    </p>
                  )}
                </div>
              )}
            </section>
          ) : null}
        </section>

        <ImageViewerModal
          open={imageViewerOpen}
          embedded={embedded}
          title={imageViewerTitle}
          src={imageViewerSrc}
          scale={imageViewerScale}
          loading={viewerImageLoading}
          onZoomOut={zoomOutImage}
          onResetZoom={resetImageZoom}
          onZoomIn={zoomInImage}
          onClose={closeImageViewer}
          onImageLoad={() => setViewerImageLoading(false)}
          onImageError={() => setViewerImageLoading(false)}
          onImageClick={() => {
            if (imageViewerScale < 2) {
              zoomInImage()
            } else {
              resetImageZoom()
            }
          }}
        />
        {deleteToastNode}
        {renameModalNode}
        {confirmDialogNode}
        {folderDialogNode}
        {shareDialogNode}
        {moveCopyDialogNode}
      </main>
    )
  }

  return (
    <main
      className={`bucky-file-app relative bg-[radial-gradient(circle_at_top,#d9eeea,transparent_58%),#f4f8f7] px-3 py-4 md:px-6 md:py-6 ${
        embedded ? 'h-full min-h-0' : 'min-h-screen'
      }`}
    >
      <div
        className={`mx-auto w-full ${
          embedded ? 'flex h-full min-h-0 max-w-none flex-col gap-4' : 'max-w-[1280px] space-y-4'
        }`}
      >
        {!embedded ? (
          <header className="rounded-[20px] border border-slate-200 bg-white/95 px-5 py-4 shadow-sm backdrop-blur">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <div>
                <p className="text-xs uppercase tracking-[0.18em] text-slate-500">Bucky Drive</p>
                <h1 className="text-2xl font-semibold text-slate-900">bucky-file</h1>
              </div>
              <div className="flex items-center gap-2">
                <span className="rounded-full border border-slate-200 bg-slate-50 px-3 py-1 text-xs font-semibold text-slate-600">
                  User: {currentUserName || 'Signed in'}
                </span>
              </div>
            </div>
          </header>
        ) : null}

        <section
          className={`overflow-hidden rounded-[24px] border border-slate-200 bg-white shadow-sm ${
            embedded && mainTab === 'files' ? 'flex min-h-0 flex-1 flex-col' : ''
          }`}
        >
          <div className="border-b border-slate-200 bg-white px-5">
            <div className="-mb-px flex flex-wrap items-center justify-between gap-2 py-1.5">
              <div className="flex items-center gap-4">
                <button
                  type="button"
                  onClick={() => navigateToMainTab('files')}
                  className={`border-b-2 px-1 py-2 text-sm font-semibold transition ${
                    mainTab === 'files'
                      ? 'border-primary text-primary'
                      : 'border-transparent text-slate-500 hover:text-primary'
                  }`}
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="folder" className="size-3.5" />
                    Files
                  </span>
                </button>
                <button
                  type="button"
                  onClick={() => navigateToMainTab('shares')}
                  className={`border-b-2 px-1 py-2 text-sm font-semibold transition ${
                    mainTab === 'shares'
                      ? 'border-primary text-primary'
                      : 'border-transparent text-slate-500 hover:text-primary'
                  }`}
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="share" className="size-3.5" />
                    Shares
                  </span>
                </button>
              </div>

              {mainTab === 'files' ? (
                <div className="flex flex-wrap items-center gap-1.5">
                  {([
                    { key: 'browse', label: 'Browse', icon: 'folder' as const },
                    { key: 'recent', label: 'Recent', icon: 'scope-recent' as const },
                    { key: 'starred', label: 'Starred', icon: 'scope-starred' as const },
                    { key: 'trash', label: 'Trash', icon: 'scope-trash' as const },
                  ] as const).map((scope) => (
                    <button
                      key={scope.key}
                      type="button"
                      onClick={() => setFilesScope(scope.key)}
                      className={`rounded-full border px-2.5 py-1 text-[11px] font-semibold transition ${
                        filesScope === scope.key
                          ? 'border-primary bg-primary text-white'
                          : 'border-slate-300 bg-white text-slate-700 hover:border-primary hover:text-primary'
                      }`}
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name={scope.icon} className="size-3.5" />
                        {scope.label}
                      </span>
                    </button>
                  ))}
                </div>
              ) : null}
            </div>
          </div>

          {mainTab === 'files' ? (
            <div className={embedded ? 'flex min-h-0 flex-1 flex-col' : ''}>
              <header className="border-b border-slate-200 px-5 py-1.5">
                <div className="flex flex-wrap items-center gap-1.5">
                  {filesScope === 'browse' ? (
                    <>
                      <HoverTooltip label="Upload">
                        <label className={`${compactPrimaryToolbarButtonClass} cursor-pointer`}>
                          <Icon name="upload" className="size-3.5" />
                          <input type="file" multiple onChange={onUpload} className="hidden" />
                        </label>
                      </HoverTooltip>
                      <HoverTooltip label="Add folder">
                        <button
                          type="button"
                          onClick={() => void onCreateFolder()}
                          className={compactSecondaryToolbarButtonClass}
                          aria-label="Add folder"
                        >
                          <Icon name="new-folder" className="size-3.5" />
                        </button>
                      </HoverTooltip>
                    </>
                  ) : null}
                  <HoverTooltip label={showAdvancedFilters ? 'Hide filters' : 'Advanced filters'}>
                    <button
                      type="button"
                      onClick={() => setShowAdvancedFilters((prev) => !prev)}
                      className={compactSecondaryToolbarButtonClass}
                      aria-label={showAdvancedFilters ? 'Hide filters' : 'Advanced filters'}
                    >
                      <Icon name="filter" className="size-3.5" />
                    </button>
                  </HoverTooltip>
                  {filesScope === 'browse' && currentPathIsDir ? (
                    <div className="inline-flex items-center overflow-hidden rounded-lg border border-primary">
                      <HoverTooltip label="Icon view">
                        <button
                          type="button"
                          onClick={() => setFilesViewMode('icon')}
                          className={`px-2.5 py-1.5 text-xs font-semibold transition ${
                            filesViewMode === 'icon'
                              ? 'bg-primary text-white'
                              : 'bg-white text-slate-700 hover:bg-slate-50 hover:text-primary'
                          }`}
                          aria-label="Icon view"
                        >
                          <span className="inline-flex items-center">
                            <Icon name="view-icon" />
                          </span>
                        </button>
                      </HoverTooltip>
                      <HoverTooltip label="List view">
                        <button
                          type="button"
                          onClick={() => setFilesViewMode('list')}
                          className={`border-l border-primary px-2.5 py-1.5 text-xs font-semibold transition ${
                            filesViewMode === 'list'
                              ? 'bg-primary text-white'
                              : 'bg-white text-slate-700 hover:bg-slate-50 hover:text-primary'
                          }`}
                          aria-label="List view"
                        >
                          <span className="inline-flex items-center">
                            <Icon name="view-list" />
                          </span>
                        </button>
                      </HoverTooltip>
                    </div>
                  ) : null}
                  <HoverTooltip label="Move selected">
                    <button
                      type="button"
                      disabled={selectedEntries.length === 0}
                      onClick={() => void onBatchMoveOrCopy('move')}
                      className={compactToolbarButtonClass}
                      aria-label="Move selected"
                    >
                      <span className="inline-flex items-center justify-center">
                        <Icon name="move" className="size-4" />
                      </span>
                    </button>
                  </HoverTooltip>
                  <HoverTooltip label="Copy selected">
                    <button
                      type="button"
                      disabled={selectedEntries.length === 0}
                      onClick={() => void onBatchMoveOrCopy('copy')}
                      className={compactToolbarButtonClass}
                      aria-label="Copy selected"
                    >
                      <span className="inline-flex items-center justify-center">
                        <Icon name="copy" className="size-4" />
                      </span>
                    </button>
                  </HoverTooltip>
                  <HoverTooltip label="Delete selected">
                    <button
                      type="button"
                      disabled={selectedEntries.length === 0}
                      onClick={() => void onBatchDelete()}
                      className={`${compactToolbarButtonClass} text-rose-600 hover:bg-rose-50 hover:text-rose-700 disabled:hover:bg-transparent disabled:hover:text-rose-600`}
                      aria-label="Delete selected"
                    >
                      <span className="inline-flex items-center justify-center">
                        <Icon name="delete" className="size-4" />
                      </span>
                    </button>
                  </HoverTooltip>
                  <HoverTooltip label="Refresh">
                    <button
                      type="button"
                      onClick={() => {
                        clearSearchState()
                        if (filesScope === 'browse') {
                          void loadDirectory(currentPath, effectiveToken)
                        } else if (filesScope === 'recent') {
                          void loadRecent(effectiveToken)
                        } else if (filesScope === 'starred') {
                          void loadFavorites(effectiveToken)
                        } else {
                          void loadTrash(effectiveToken)
                        }
                      }}
                      className={compactSecondaryToolbarButtonClass}
                      aria-label="Refresh"
                    >
                      <Icon name="retry" className="size-3.5" />
                    </button>
                  </HoverTooltip>
                  <input
                    value={searchKeyword}
                    onChange={(event) => setSearchKeyword(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault()
                        void onSearch()
                      }
                    }}
                    placeholder={filesScope === 'browse' ? 'Search by file name or path' : 'Search in current view'}
                    className="w-full min-w-[220px] max-w-[720px] flex-1 rounded-lg border border-slate-300 bg-slate-50 px-3 py-1 text-sm text-slate-700 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20"
                  />
                  <HoverTooltip label={searchLoading && filesScope === 'browse' ? 'Searching...' : 'Search'}>
                    <button
                      type="button"
                      onClick={() => void onSearch()}
                      disabled={searchLoading && filesScope === 'browse'}
                      className={compactPrimaryToolbarButtonClass}
                      aria-label={searchLoading && filesScope === 'browse' ? 'Searching...' : 'Search'}
                    >
                      <Icon name="search" className="size-3.5" />
                    </button>
                  </HoverTooltip>
                  {searchActive ? (
                    <HoverTooltip label="Clear search">
                      <button
                        type="button"
                        onClick={onClearSearch}
                        className={compactSecondaryToolbarButtonClass}
                        aria-label="Clear search"
                      >
                        <Icon name="clear" className="size-3.5" />
                      </button>
                    </HoverTooltip>
                  ) : null}
                </div>

                {showAdvancedFilters ? (
                  <div className="mt-2 grid gap-2 rounded-xl border border-slate-200 bg-slate-50 p-3 md:grid-cols-4">
                    <label className="text-xs font-semibold text-slate-600">
                      Kind
                      <select
                        value={filterKind}
                        onChange={(event) => setFilterKind(event.target.value as 'all' | 'file' | 'dir')}
                        className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                      >
                        <option value="all">All</option>
                        <option value="file">Files</option>
                        <option value="dir">Folders</option>
                      </select>
                    </label>
                    <label className="text-xs font-semibold text-slate-600">
                      Extension (csv)
                      <input
                        value={filterExtInput}
                        onChange={(event) => setFilterExtInput(event.target.value)}
                        placeholder="jpg,png,pdf"
                        className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                      />
                    </label>
                    <label className="text-xs font-semibold text-slate-600">
                      Modified
                      <select
                        value={filterDatePreset}
                        onChange={(event) => setFilterDatePreset(event.target.value as 'all' | '7d' | '30d' | 'custom')}
                        className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                      >
                        <option value="all">Any time</option>
                        <option value="7d">Last 7 days</option>
                        <option value="30d">Last 30 days</option>
                        <option value="custom">Custom range</option>
                      </select>
                    </label>
                    <label className="text-xs font-semibold text-slate-600">
                      Sort
                      <div className="mt-1 flex gap-2">
                        <select
                          value={filterSortBy}
                          onChange={(event) => setFilterSortBy(event.target.value as 'name' | 'modified' | 'size')}
                          className="w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                        >
                          <option value="name">Name</option>
                          <option value="modified">Modified</option>
                          <option value="size">Size</option>
                        </select>
                        <select
                          value={filterSortOrder}
                          onChange={(event) => setFilterSortOrder(event.target.value as 'asc' | 'desc')}
                          className="w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                        >
                          <option value="asc">Asc</option>
                          <option value="desc">Desc</option>
                        </select>
                      </div>
                    </label>

                    {filterDatePreset === 'custom' ? (
                      <>
                        <label className="text-xs font-semibold text-slate-600">
                          From
                          <input
                            type="datetime-local"
                            value={filterDateFrom}
                            onChange={(event) => setFilterDateFrom(event.target.value)}
                            className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                          />
                        </label>
                        <label className="text-xs font-semibold text-slate-600">
                          To
                          <input
                            type="datetime-local"
                            value={filterDateTo}
                            onChange={(event) => setFilterDateTo(event.target.value)}
                            className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                          />
                        </label>
                      </>
                    ) : null}

                    <label className="text-xs font-semibold text-slate-600">
                      Size min (MB)
                      <input
                        type="number"
                        min="0"
                        value={filterSizeMin}
                        onChange={(event) => setFilterSizeMin(event.target.value)}
                        className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                      />
                    </label>
                    <label className="text-xs font-semibold text-slate-600">
                      Size max (MB)
                      <input
                        type="number"
                        min="0"
                        value={filterSizeMax}
                        onChange={(event) => setFilterSizeMax(event.target.value)}
                        className="mt-1 w-full rounded-lg border border-slate-300 bg-white px-2 py-1.5 text-xs text-slate-700"
                      />
                    </label>
                    <div className="flex items-end">
                      <button
                        type="button"
                        onClick={() => {
                          setFilterKind('all')
                          setFilterExtInput('')
                          setFilterDatePreset('all')
                          setFilterDateFrom('')
                          setFilterDateTo('')
                          setFilterSizeMin('')
                          setFilterSizeMax('')
                          setFilterSortBy('name')
                          setFilterSortOrder('asc')
                        }}
                        className="rounded-lg border border-slate-300 px-3 py-1.5 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                      >
                        Reset filters
                      </button>
                    </div>
                  </div>
                ) : null}
              </header>

              <div className="border-b border-slate-200 px-5 py-2">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <div className="min-w-0 flex flex-wrap items-center gap-2">
                    {filesScope === 'browse' ? (
                      <div className="inline-flex min-w-0 max-w-full items-center overflow-hidden rounded-full bg-slate-100 px-1 py-0.5 text-[11px] font-semibold text-slate-600">
                        <button
                          type="button"
                          onClick={() => openDirectory('/')}
                          className="rounded-full px-2 py-0.5 text-slate-700 transition hover:bg-white hover:text-primary"
                        >
                          Root
                        </button>
                        {currentPathSegments.map((segment, index) => {
                          const partialPath = `/${currentPathSegments.slice(0, index + 1).join('/')}`
                          const isLast = index === currentPathSegments.length - 1

                          return (
                            <span key={partialPath} className="inline-flex min-w-0 items-center">
                              <span className="px-1 text-slate-400">/</span>
                              <button
                                type="button"
                                onClick={() => openDirectory(partialPath)}
                                disabled={isLast}
                                className={`max-w-[120px] truncate rounded-full px-2 py-0.5 transition ${
                                  isLast
                                    ? 'cursor-default text-slate-500'
                                    : 'text-slate-700 hover:bg-white hover:text-primary'
                                }`}
                                title={segment}
                              >
                                {segment}
                              </button>
                            </span>
                          )
                        })}
                      </div>
                    ) : (
                      <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-[11px] font-semibold text-slate-600">
                        View: {filesScope}
                      </span>
                    )}
                  </div>

                  <div className="ml-auto flex flex-wrap items-center justify-end gap-2">
                    {filesScope === 'browse' ? (
                      currentPathIsDir ? (
                        <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-[11px] font-semibold text-slate-600">
                          {visibleFolderCount} folders · {visibleFileCount} files
                        </span>
                      ) : (
                        <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-[11px] font-semibold text-slate-600">File preview mode</span>
                      )
                    ) : (
                      <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-[11px] font-semibold text-slate-600">
                        {visibleItems.length} items
                      </span>
                    )}
                    {loading ? <span className="rounded-full bg-slate-100 px-2.5 py-0.5 text-[11px] font-semibold text-slate-600">Working...</span> : null}
                    {searchActive ? (
                      <span className="rounded-full bg-amber-50 px-2.5 py-0.5 text-[11px] font-semibold text-amber-700">
                        {searchResults.length} result(s){searchTruncated ? ' · truncated' : ''}
                      </span>
                    ) : null}
                    {selectedEntries.length > 0 ? (
                      <span className="rounded-full bg-primary/10 px-2.5 py-0.5 text-[11px] font-semibold text-primary">
                        {selectedEntries.length} selected
                      </span>
                    ) : null}
                  </div>
                </div>
              </div>

          {message ? (
            <p className="border-b border-slate-200 bg-slate-50 px-5 py-3 text-sm font-medium text-slate-700">{message}</p>
          ) : null}

          {currentPathIsDir ? (
              <div
                className={`relative px-3 py-3 transition md:px-4 ${
                  embedded ? 'min-h-0 flex-1 overflow-auto' : 'overflow-x-auto'
                } ${
                  filesScope === 'browse' && dropzoneActive ? 'bg-teal-50/70 ring-2 ring-primary/25 ring-inset' : ''
                }`}
                onDragEnter={filesScope === 'browse' ? onListDragEnter : undefined}
                onDragOver={filesScope === 'browse' ? onListDragOver : undefined}
                onDragLeave={filesScope === 'browse' ? onListDragLeave : undefined}
                onDrop={filesScope === 'browse' ? (event) => void onListDrop(event) : undefined}
              >
                {filesScope === 'browse' && dropzoneActive ? (
                  <div className="pointer-events-none absolute inset-3 z-20 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary/60 bg-white/85">
                    <div className="inline-flex items-center gap-2 rounded-xl bg-white px-3 py-2 text-sm font-semibold text-primary shadow-sm">
                      <Icon name="upload" />
                      Drop files to upload
                    </div>
                  </div>
                ) : null}

                {filesViewMode === 'list' ? (
                  <table className={`w-full min-w-[760px] border-separate border-spacing-0 text-sm ${dropzoneActive ? 'opacity-60' : ''}`}>
                    <thead>
                      <tr className="text-left text-xs uppercase tracking-wide text-slate-500">
                        <th className="rounded-l-lg bg-slate-50 px-3 py-2">
                          <input
                            type="checkbox"
                            checked={allSelected}
                            onChange={toggleSelectAll}
                            className="size-4 rounded border-slate-300 text-primary focus:ring-primary"
                            aria-label="Select all"
                          />
                        </th>
                        <th className="bg-slate-50 px-3 py-2">Name</th>
                        {searchActive || filesScope !== 'browse' ? <th className="bg-slate-50 px-3 py-2">Path</th> : null}
                        <th className="bg-slate-50 px-3 py-2">Type</th>
                        <th className="bg-slate-50 px-3 py-2">Size</th>
                        <th className="rounded-r-lg bg-slate-50 px-3 py-2">Modified</th>
                      </tr>
                    </thead>
                    <tbody>
                      {visibleItems.length === 0 ? (
                        <tr>
                          <td colSpan={searchActive || filesScope !== 'browse' ? 6 : 5} className="px-3 py-12 text-center text-sm text-slate-500">
                            {searchActive ? 'No search result.' : filesScope === 'browse' ? 'Empty directory.' : 'No items in this view.'}
                          </td>
                        </tr>
                      ) : (
                        visibleItems.map((entry) => {
                          const entryIcon = getEntryIconMeta(entry)
                          return (
                          <tr
                            key={entry.path}
                            className="text-slate-800"
                            onContextMenu={(event) => onEntryContextMenu(event, entry)}
                            onPointerDown={(event) => onEntryPointerDown(event, entry)}
                            onPointerMove={onEntryPointerMove}
                            onPointerUp={onEntryPointerEnd}
                            onPointerCancel={onEntryPointerEnd}
                            onClickCapture={onEntryClickCapture}
                          >
                            <td className="border-b border-slate-100 px-3 py-2">
                              <input
                                type="checkbox"
                                checked={isSelected(entry.path)}
                                onChange={() => toggleSelection(entry.path)}
                                className="size-4 rounded border-slate-300 text-primary focus:ring-primary"
                                aria-label={`Select ${entry.name}`}
                              />
                            </td>
                            <td className="border-b border-slate-100 px-3 py-2 font-medium">
                              {entry.is_dir ? (
                                <button
                                  type="button"
                                  onClick={() => openDirectoryInBrowse(entry.path)}
                                  className="rounded px-1 py-0.5 text-left text-primary transition hover:bg-primary/10"
                                  aria-label={entry.name}
                                >
                                  <span className="inline-flex items-center gap-1.5">
                                    <Icon name="folder" className="shrink-0" />
                                    <FileNameTooltip name={entry.name} />
                                  </span>
                                </button>
                              ) : (
                                <a
                                  href={buildFileDetailPath(entry.path)}
                                  target="_blank"
                                  rel="noreferrer"
                                  className="rounded px-1 py-0.5 text-left text-slate-800 transition hover:bg-slate-100"
                                  aria-label={entry.name}
                                >
                                  <span className="inline-flex items-center gap-1.5">
                                    {canUseThumbnail(entry, 48) ? (
                                      <img
                                        src={buildThumbnailUrl(entry.path, 48)}
                                        alt=""
                                        className="size-6 shrink-0 rounded-[2px] border border-slate-200 object-cover"
                                        loading="lazy"
                                        onError={() => {
                                          setThumbLoadFailed((prev) => ({
                                            ...prev,
                                            [buildThumbnailFailureKey(entry.path, 48)]: true,
                                          }))
                                        }}
                                      />
                                    ) : (
                                      <Icon name={entryIcon.iconName} className={`${entryIcon.iconClassName ?? ''} shrink-0`.trim()} />
                                    )}
                                    <FileNameTooltip name={entry.name} />
                                  </span>
                                </a>
                              )}
                            </td>
                            {searchActive || filesScope !== 'browse' ? <td className="border-b border-slate-100 px-3 py-2 text-slate-600">{parentPath(entry.path)}</td> : null}
                            <td className="border-b border-slate-100 px-3 py-2">{entry.is_dir ? 'Folder' : 'File'}</td>
                            <td className="border-b border-slate-100 px-3 py-2">{entry.is_dir ? '-' : formatBytes(entry.size)}</td>
                            <td className="border-b border-slate-100 px-3 py-2">{formatTimestamp(entry.modified)}</td>
                          </tr>
                          )
                        })
                      )}
                    </tbody>
                  </table>
                ) : (
                  <>
                    {visibleItems.length === 0 ? (
                      <div className="rounded-xl border border-dashed border-slate-300 px-3 py-12 text-center text-sm text-slate-500">
                        {searchActive ? 'No search result.' : filesScope === 'browse' ? 'Empty directory.' : 'No items in this view.'}
                      </div>
                    ) : (
                      <div
                        className={`grid grid-cols-[repeat(auto-fill,minmax(136px,1fr))] gap-3 ${dropzoneActive ? 'opacity-60' : ''}`}
                        onClick={(event) => {
                          if (event.target === event.currentTarget) {
                            setSelectedPaths([])
                          }
                        }}
                      >
                        {visibleItems.map((entry) => {
                          const entryIcon = getEntryIconMeta(entry)
                          return (
                            <article
                              key={entry.path}
                              onClick={(event) => onIconEntryClick(event, entry)}
                              onDoubleClick={(event) => onIconEntryDoubleClick(event, entry)}
                              onContextMenu={(event) => onEntryContextMenu(event, entry)}
                              onPointerDown={(event) => onEntryPointerDown(event, entry)}
                              onPointerMove={onEntryPointerMove}
                              onPointerUp={onEntryPointerEnd}
                              onPointerCancel={onEntryPointerEnd}
                              onClickCapture={onEntryClickCapture}
                              className={`relative cursor-pointer rounded-xl p-3 transition ${
                                isSelected(entry.path)
                                  ? 'bg-primary/10 shadow-sm ring-2 ring-primary/30'
                                  : 'bg-white hover:bg-slate-50 hover:shadow-sm'
                              }`}
                            >
                              <div className="flex h-[126px] flex-col items-center justify-start pt-6 text-center">
                                {entry.is_dir ? (
                                  <div className="inline-flex size-14 items-center justify-center rounded-xl bg-amber-50 text-amber-600">
                                    <Icon name="folder" className="size-7" />
                                  </div>
                                ) : canUseThumbnail(entry, 160) ? (
                                  <img
                                    src={buildThumbnailUrl(entry.path, 160)}
                                    alt=""
                                    className="size-14 rounded-md border border-slate-200 object-cover"
                                    loading="lazy"
                                    onError={() => {
                                      setThumbLoadFailed((prev) => ({
                                        ...prev,
                                        [buildThumbnailFailureKey(entry.path, 160)]: true,
                                      }))
                                    }}
                                  />
                                ) : (
                                  <div className="inline-flex size-14 items-center justify-center rounded-xl bg-slate-50">
                                    <Icon
                                      name={entryIcon.iconName}
                                      className={`${entryIcon.iconClassName ?? ''} size-7`.trim()}
                                    />
                                  </div>
                                )}
                                <div className={`mt-2 w-full font-medium ${entry.is_dir ? 'text-primary' : 'text-slate-800'}`}>
                                  <FileNameTooltip name={entry.name} maxChars={28} maxWidthClass="max-w-full" />
                                </div>
                                {searchActive || filesScope !== 'browse' ? <p className="mt-1 w-full truncate text-[11px] text-slate-500">{parentPath(entry.path)}</p> : null}
                              </div>
                            </article>
                          )
                        })}
                      </div>
                    )}
                  </>
                )}
              </div>
          ) : null}

          {openActionEntry && actionMenuPosition && typeof document !== 'undefined'
            ? createPortal(
                <div
                  ref={rowActionMenuRef}
                  className="fixed z-[60] w-44 overflow-hidden rounded-xl border border-slate-200 bg-white shadow-xl"
                  style={{ top: actionMenuPosition.top, left: actionMenuPosition.left }}
                  data-row-actions="true"
                >
                  {filesScope === 'trash' ? (
                    <>
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          if (openActionTrashItem) {
                            void restoreTrashItem(openActionTrashItem)
                          }
                        }}
                        className={rowActionItemClass}
                      >
                        <Icon name="restore" />
                        Restore
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          if (openActionTrashItem) {
                            void deleteTrashItemForever(openActionTrashItem)
                          }
                        }}
                        className={`${rowActionItemClass} text-rose-600 hover:bg-rose-50 hover:text-rose-700`}
                      >
                        <Icon name="delete" />
                        Delete forever
                      </button>
                    </>
                  ) : (
                    <>
                      {!openActionEntry.is_dir ? (
                        <a
                          href={buildFileDetailPath(openActionEntry.path)}
                          target="_blank"
                          rel="noreferrer"
                          className={rowActionItemClass}
                          onClick={closeRowActionMenu}
                        >
                          <Icon name="open" />
                          Open
                        </a>
                      ) : null}
                      <a
                        href={buildRawFileUrl(openActionEntry.path, true)}
                        className={rowActionItemClass}
                        onClick={closeRowActionMenu}
                      >
                        <Icon name="download" />
                        Download
                      </a>
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          void toggleFavorite(openActionEntry)
                        }}
                        className={rowActionItemClass}
                      >
                        <Icon name="star" />
                        {favoritePathSet.has(openActionEntry.path) ? 'Unstar' : 'Star'}
                      </button>
                      {!openActionEntry.is_dir ? (
                        <button
                          type="button"
                          onClick={() => {
                            closeRowActionMenu()
                            void onOpenEditor(openActionEntry)
                          }}
                          className={rowActionItemClass}
                        >
                          <Icon name="open" />
                          Edit
                        </button>
                      ) : null}
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          void onRename(openActionEntry)
                        }}
                        className={rowActionItemClass}
                      >
                        <Icon name="rename" />
                        Rename
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          void onMoveOrCopy(openActionEntry, 'move')
                        }}
                        className={rowActionItemClass}
                      >
                        <Icon name="move" />
                        Move
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          void onMoveOrCopy(openActionEntry, 'copy')
                        }}
                        className={rowActionItemClass}
                      >
                        <Icon name="copy" />
                        Copy
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          void onCreateShare(openActionEntry)
                        }}
                        className={`${rowActionItemClass} text-amber-700 hover:bg-amber-50 hover:text-amber-700`}
                      >
                        <Icon name="share" />
                        Share
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          closeRowActionMenu()
                          void onDelete(openActionEntry)
                        }}
                        className={`${rowActionItemClass} text-rose-600 hover:bg-rose-50 hover:text-rose-700`}
                      >
                        <Icon name="delete" />
                        Delete
                      </button>
                    </>
                  )}
                </div>,
                document.body,
              )
            : null}

          {filesScope === 'browse' ? (
            <FilePreviewPanel
              embedded={embedded}
              previewEntry={previewEntry}
              previewKind={displayPreviewKind}
              previewRawUrl={displayPreviewRawUrl}
              previewImageSrc={previewImageSrc}
              previewLoading={previewLoading}
              previewLoadingLabel={previewLoadingLabel}
              previewLoadingElapsedSeconds={previewLoadingElapsedSeconds}
              previewError={previewError}
              previewTextContent={previewTextContent}
              previewDocxHtml={previewDocxHtml}
              previewImageLoading={previewImageLoading}
              previewImageProgressPercent={previewImageProgressPercent}
              previewImageLoadedBytes={previewImageLoadedBytes}
              previewImageTotalBytes={previewImageTotalBytes}
              officePreviewUrl={officePreviewUrl}
              onOpenImageViewer={openImageViewer}
              onPreviewImageLoad={() => setPreviewImageLoading(false)}
              onPreviewImageError={() => {
                setPreviewImageLoading(false)
                setPreviewError('Image preview failed to render. Please retry.')
              }}
              formatBytes={formatBytes}
              formatTimestamp={formatTimestamp}
            />
          ) : null}

            </div>
          ) : null}

          {mainTab === 'shares' ? (
            <div className="border-t border-slate-200 bg-slate-50 px-5 py-4">
              <section className="rounded-2xl border border-slate-200 bg-white p-4">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <p className="text-sm font-semibold text-slate-800">Share links</p>
                  <p className="text-xs text-slate-500">Manage links for quick collaboration.</p>
                </div>
                {sharesLoading ? <span className="text-xs text-slate-500">Loading...</span> : null}
              </div>

              {shares.length === 0 ? (
                <p className="mt-3 rounded-lg border border-dashed border-slate-300 bg-slate-50 px-3 py-2 text-sm text-slate-500">
                  No share links yet.
                </p>
              ) : (
                <div className="mt-3 space-y-2">
                  {shares.map((share) => (
                    <div key={share.id} className="rounded-xl border border-slate-200 bg-slate-50 px-3 py-2">
                      <div className="flex flex-wrap items-center gap-2">
                        <p className="text-sm font-semibold text-slate-800">{share.path}</p>
                        <span className="rounded-full bg-white px-2 py-0.5 text-[11px] font-semibold text-slate-600">
                          {share.password_required ? 'Password protected' : 'Public'}
                        </span>
                        <span className="rounded-full bg-white px-2 py-0.5 text-[11px] font-semibold text-slate-600">
                          {share.expires_at ? `Expires ${formatTimestamp(share.expires_at)}` : 'No expiration'}
                        </span>
                      </div>
                      <div className="mt-2 flex flex-wrap items-center gap-2">
                        <button
                          type="button"
                          onClick={() => void onCopyShareLink(share.id, 'view')}
                          className="rounded border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                        >
                          <span className="inline-flex items-center gap-1.5">
                            <Icon name="link" />
                            Copy view link
                          </span>
                        </button>
                        <button
                          type="button"
                          onClick={() => void onCopyShareLink(share.id, 'download')}
                          className="rounded border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                        >
                          <span className="inline-flex items-center gap-1.5">
                            <Icon name="link" />
                            Copy download link
                          </span>
                        </button>
                        <a
                          href={buildPublicSharePath(share.id)}
                          target="_blank"
                          rel="noreferrer"
                          className="rounded border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                        >
                          Open
                        </a>
                        <button
                          type="button"
                          onClick={() => void onDeleteShare(share.id)}
                          className="rounded border border-rose-300 px-2 py-1 text-xs font-semibold text-rose-600 transition hover:bg-rose-50"
                        >
                          <span className="inline-flex items-center gap-1.5">
                            <Icon name="delete" />
                            Remove
                          </span>
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
              </section>
            </div>
          ) : null}

          {mainTab === 'editor' ? (
            <div className="border-t border-slate-200 bg-slate-50 px-5 py-4">
              <section className="rounded-2xl border border-slate-200 bg-white p-4">
              {editorPath ? (
                <>
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <p className="text-sm font-semibold text-slate-800">Editor: {editorPath}</p>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        onClick={() => void onSaveEditor()}
                        disabled={!editorDirty || editorSaving}
                        className="rounded-lg bg-primary px-3 py-2 text-xs font-semibold text-white transition hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-60"
                      >
                        <span className="inline-flex items-center gap-1.5">
                          <Icon name="save" />
                          {editorSaving ? 'Saving...' : 'Save'}
                        </span>
                      </button>
                      <button
                        type="button"
                        onClick={onCloseEditor}
                        className="rounded-lg border border-slate-300 px-3 py-2 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                      >
                        <span className="inline-flex items-center gap-1.5">
                          <Icon name="close" />
                          Close
                        </span>
                      </button>
                    </div>
                  </div>
                  <textarea
                    value={editorContent}
                    onChange={(event) => {
                      setEditorContent(event.target.value)
                      setEditorDirty(true)
                    }}
                    className="mt-3 min-h-[260px] w-full rounded-xl border border-slate-300 bg-slate-50 p-3 text-sm text-slate-800 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20"
                    spellCheck={false}
                  />
                </>
              ) : (
                <div className="flex min-h-[220px] flex-col items-center justify-center rounded-xl border border-dashed border-slate-300 bg-slate-50 px-4 text-center">
                  <p className="text-sm font-semibold text-slate-700">No file in editor</p>
                  <p className="mt-1 text-xs text-slate-500">Select any text file and click Edit to start.</p>
                </div>
              )}
              </section>
            </div>
          ) : null}
        </section>

        <div className={`${embedded ? 'absolute' : 'fixed'} bottom-5 right-5 z-40 flex max-h-[78vh] w-[min(92vw,420px)] flex-col items-end gap-2`}>
          {uploadPanelOpen ? (
            <section className="max-h-[64vh] w-full overflow-hidden rounded-2xl border border-slate-200 bg-white shadow-xl">
              <div className="flex items-center justify-between gap-2 border-b border-slate-200 px-3 py-2">
                <p className="text-sm font-semibold text-slate-800">
                  Upload queue {activeUploadCount > 0 ? `(${activeUploadCount} active)` : ''}
                </p>
                <div className="flex items-center gap-2">
                  {activeUploadCount > 0 ? (
                    <button
                      type="button"
                      onClick={() => {
                        const next = !uploadPausedRef.current
                        uploadPausedRef.current = next
                        setUploadPaused(next)
                      }}
                      className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name={uploadPaused ? 'resume' : 'pause'} />
                        {uploadPaused ? 'Resume' : 'Pause'}
                      </span>
                    </button>
                  ) : null}
                  <button
                    type="button"
                    onClick={onClearCompletedUploads}
                    className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                  >
                    <span className="inline-flex items-center gap-1.5">
                      <Icon name="clear" />
                      Clear
                    </span>
                  </button>
                </div>
              </div>

              <div className="max-h-[52vh] space-y-2 overflow-auto bg-slate-50/70 p-3">
                {uploadProgress.length === 0 ? (
                  <div className="rounded-xl border border-dashed border-slate-300 bg-white px-3 py-4 text-center text-sm text-slate-500">
                    No uploads yet.
                  </div>
                ) : (
                  uploadProgress.map((item) => {
                    const progress = item.total > 0 ? Math.round((item.uploaded / item.total) * 100) : 100
                    return (
                      <div key={item.key} className="rounded-xl border border-slate-200 bg-white px-3 py-2">
                        <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
                          <span className="font-semibold text-slate-800">{item.name}</span>
                          <span
                            className={`font-semibold ${
                              item.status === 'error'
                                ? 'text-rose-600'
                                : item.status === 'paused'
                                  ? 'text-amber-700'
                                : item.status === 'cancelled'
                                  ? 'text-slate-600'
                                : item.status === 'completed'
                                  ? 'text-emerald-700'
                                  : 'text-slate-600'
                            }`}
                          >
                            {item.status === 'uploading'
                              ? `Uploading ${progress}%`
                              : item.status === 'paused'
                                ? `Paused ${progress}%`
                              : item.status === 'cancelled'
                                ? 'Cancelled'
                              : item.status === 'completed'
                                ? 'Completed'
                                : 'Failed'}
                          </span>
                        </div>
                        <div className="mt-2 h-2 overflow-hidden rounded-full bg-slate-200">
                          <div
                            className={`h-full rounded-full transition-all ${
                              item.status === 'error'
                                ? 'bg-rose-500'
                                : item.status === 'paused'
                                  ? 'bg-amber-500'
                                  : item.status === 'cancelled'
                                    ? 'bg-slate-400'
                                    : 'bg-primary'
                            }`}
                            style={{ width: `${Math.max(0, Math.min(progress, 100))}%` }}
                          />
                        </div>
                        <div className="mt-1 text-[11px] text-slate-500">
                          {formatBytes(item.uploaded)} / {formatBytes(item.total)}
                        </div>
                        <div className="mt-2 flex items-center gap-2">
                          {item.status === 'uploading' || item.status === 'paused' ? (
                            <button
                              type="button"
                              onClick={() => void onCancelUpload(item.key)}
                              className="rounded border border-rose-300 px-2 py-1 text-[11px] font-semibold text-rose-600 transition hover:bg-rose-50"
                            >
                              <span className="inline-flex items-center gap-1.5">
                                <Icon name="close" />
                                Cancel
                              </span>
                            </button>
                          ) : null}
                          {item.status === 'error' || item.status === 'cancelled' ? (
                            <button
                              type="button"
                              onClick={() => void onRetryUpload(item.key)}
                              className="rounded border border-slate-300 px-2 py-1 text-[11px] font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                            >
                              <span className="inline-flex items-center gap-1.5">
                                <Icon name="retry" />
                                Retry
                              </span>
                            </button>
                          ) : null}
                        </div>
                        {item.error ? <p className="mt-1 text-[11px] text-rose-600">{item.error}</p> : null}
                      </div>
                    )
                  })
                )}
              </div>
            </section>
          ) : null}

          <button
            type="button"
            onClick={() => setUploadPanelOpen((prev) => !prev)}
            className="min-h-8 rounded-xl border border-white/25 bg-primary/65 px-2.5 py-1.5 text-xs font-semibold text-white shadow-md shadow-teal-900/20 backdrop-blur-md transition hover:bg-primary/80"
          >
            <span className="inline-flex items-center gap-1.5">
              <Icon name="upload" className="size-3.5" />
              Uploads
              <span className="rounded-full bg-white/20 px-1.5 py-0 text-[11px] font-semibold">{uploadQueueCount}</span>
            </span>
          </button>
        </div>

        <ImageViewerModal
          open={imageViewerOpen}
          embedded={embedded}
          title={imageViewerTitle}
          src={imageViewerSrc}
          scale={imageViewerScale}
          loading={viewerImageLoading}
          onZoomOut={zoomOutImage}
          onResetZoom={resetImageZoom}
          onZoomIn={zoomInImage}
          onClose={closeImageViewer}
          onImageLoad={() => setViewerImageLoading(false)}
          onImageError={() => setViewerImageLoading(false)}
          onImageClick={() => {
            if (imageViewerScale < 2) {
              zoomInImage()
            } else {
              resetImageZoom()
            }
          }}
        />
        {deleteToastNode}
        {renameModalNode}
        {confirmDialogNode}
        {folderDialogNode}
        {shareDialogNode}
        {moveCopyDialogNode}
      </div>
    </main>
  )
}

export default FileManagerPage
