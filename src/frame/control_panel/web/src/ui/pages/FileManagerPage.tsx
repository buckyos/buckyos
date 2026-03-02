import type { ChangeEventHandler, DragEvent, MouseEvent as ReactMouseEvent } from 'react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import {
  ArrowUp,
  Copy,
  Download,
  ExternalLink,
  FilePlus2,
  Folder,
  FolderPlus,
  Link,
  MoreHorizontal,
  Move,
  Pause,
  PencilLine,
  Play,
  RotateCw,
  Save,
  Search,
  Share2,
  Trash2,
  Upload,
  type LucideIcon,
  X,
  ZoomIn,
  ZoomOut,
} from 'lucide-react'

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

type SearchResponse = {
  query: string
  path: string
  kind: 'all' | 'file' | 'dir'
  limit: number
  truncated: boolean
  items: FileEntry[]
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

const TOKEN_STORAGE_KEY = 'bucky-file-token'
const DEFAULT_UPLOAD_CHUNK_SIZE = 2 * 1024 * 1024
const UPLOAD_CONCURRENCY = 2
const UPLOAD_MAX_RETRY = 3
const UPLOAD_RETRY_BASE_DELAY_MS = 600
const PUBLIC_TEXT_PREVIEW_LIMIT = 200_000
const ROW_ACTION_MENU_WIDTH = 176
const ROW_ACTION_MENU_ESTIMATED_HEIGHT = 292
const ROW_ACTION_MENU_GAP = 6
const ROW_ACTION_MENU_VIEWPORT_PADDING = 8

const getPublicShareIdFromPath = (path: string) => {
  const normalizedPath = path.endsWith('/') && path !== '/' ? path.slice(0, -1) : path
  const match = normalizedPath.match(/^\/share\/([^/]+)$/)
  return match?.[1] ? decodeURIComponent(match[1]) : ''
}

const getSearchParam = (key: string) => new URLSearchParams(window.location.search).get(key) ?? ''

const getCookieAuthToken = () => {
  if (typeof document === 'undefined') {
    return ''
  }
  const matched = document.cookie.match(/(?:^|;\s*)auth=([^;]+)/)
  if (!matched?.[1]) {
    return ''
  }
  try {
    return decodeURIComponent(matched[1])
  } catch {
    return matched[1]
  }
}

const withAuthHeaders = (authToken: string, extraHeaders?: Record<string, string>) => {
  const headers: Record<string, string> = {
    ...(extraHeaders ?? {}),
  }
  if (authToken.trim()) {
    headers['X-Auth'] = authToken.trim()
  }
  return headers
}

const buildPublicSharePath = (shareId: string) => `/share/${encodeURIComponent(shareId)}`

const buildPublicDownloadPath = (shareId: string, password?: string) => {
  const query = password?.trim()
    ? `?password=${encodeURIComponent(password.trim())}`
    : ''
  return `/api/public/dl/${encodeURIComponent(shareId)}${query}`
}

const buildPublicDownloadPathForTarget = (shareId: string, targetPath: string, password?: string) => {
  const query = new URLSearchParams()
  if (password?.trim()) {
    query.set('password', password.trim())
  }
  if (targetPath && targetPath !== '/') {
    query.set('path', targetPath)
  }
  const suffix = query.toString()
  return `/api/public/dl/${encodeURIComponent(shareId)}${suffix ? `?${suffix}` : ''}`
}

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

type MainTab = 'files' | 'shares' | 'editor'

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

type FilePreviewKind = 'image' | 'pdf' | 'text' | 'office' | 'unknown'

const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg'])
const TEXT_DOCUMENT_EXTENSIONS = new Set([
  'txt',
  'md',
  'markdown',
  'json',
  'yaml',
  'yml',
  'toml',
  'ini',
  'conf',
  'log',
  'csv',
  'xml',
])
const OFFICE_DOCUMENT_EXTENSIONS = new Set(['doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'odt', 'ods', 'odp'])

const getFileExtension = (name: string) => {
  const dot = name.lastIndexOf('.')
  if (dot < 0 || dot === name.length - 1) {
    return ''
  }
  return name.slice(dot + 1).toLowerCase()
}

const getFilePreviewKind = (entry: FileEntry): FilePreviewKind => {
  const ext = getFileExtension(entry.name)
  if (IMAGE_EXTENSIONS.has(ext)) {
    return 'image'
  }
  if (ext === 'pdf') {
    return 'pdf'
  }
  if (TEXT_DOCUMENT_EXTENSIONS.has(ext)) {
    return 'text'
  }
  if (OFFICE_DOCUMENT_EXTENSIONS.has(ext)) {
    return 'office'
  }
  return 'unknown'
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
  | 'more'
  | 'zoom-in'
  | 'zoom-out'
  | 'zoom-reset'

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
    more: MoreHorizontal,
    save: Save,
    close: X,
    folder: Folder,
    'zoom-in': ZoomIn,
    'zoom-out': ZoomOut,
    'zoom-reset': RotateCw,
  }

  const Lucide = icons[name]
  return <Lucide className={`size-4 shrink-0 ${className}`} aria-hidden />
}

type FileManagerPageProps = {
  embedded?: boolean
}

const FileManagerPage = ({ embedded = false }: FileManagerPageProps) => {
  const [token, setToken] = useState(() => localStorage.getItem(TOKEN_STORAGE_KEY) || getCookieAuthToken() || '')
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
  const [items, setItems] = useState<FileEntry[]>([])
  const [loading, setLoading] = useState(false)
  const [message, setMessage] = useState('')
  const [selectedPaths, setSelectedPaths] = useState<string[]>([])
  const [shares, setShares] = useState<ShareItem[]>([])
  const [sharesLoading, setSharesLoading] = useState(false)
  const [editorPath, setEditorPath] = useState('')
  const [editorContent, setEditorContent] = useState('')
  const [editorDirty, setEditorDirty] = useState(false)
  const [editorSaving, setEditorSaving] = useState(false)
  const [searchKeyword, setSearchKeyword] = useState('')
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
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewError, setPreviewError] = useState('')
  const [imageViewerOpen, setImageViewerOpen] = useState(false)
  const [imageViewerSrc, setImageViewerSrc] = useState('')
  const [imageViewerTitle, setImageViewerTitle] = useState('')
  const [imageViewerScale, setImageViewerScale] = useState(1)
  const [previewImageLoading, setPreviewImageLoading] = useState(false)
  const [publicImageLoading, setPublicImageLoading] = useState(false)
  const [viewerImageLoading, setViewerImageLoading] = useState(false)
  const [openActionPath, setOpenActionPath] = useState('')
  const [actionMenuPosition, setActionMenuPosition] = useState<{ top: number; left: number } | null>(null)

  const effectiveToken = token || getCookieAuthToken()
  const uploadPausedRef = useRef(false)
  const dropDragDepthRef = useRef(0)
  const uploadFilesRef = useRef(new Map<string, { file: File; targetPath: string; resumeKey: string }>())
  const uploadSessionRef = useRef(new Map<string, string>())
  const uploadCancelledRef = useRef(new Set<string>())
  const rowActionMenuRef = useRef<HTMLDivElement | null>(null)

  const closeRowActionMenu = useCallback(() => {
    setOpenActionPath('')
    setActionMenuPosition(null)
  }, [])

  const clearSearchState = useCallback(() => {
    setSearchActive(false)
    setSearchResults([])
    setSearchTruncated(false)
  }, [])

  const clearPreviewState = useCallback(() => {
    setPreviewEntry(null)
    setPreviewKind('unknown')
    setPreviewTextContent('')
    setPreviewError('')
    setPreviewLoading(false)
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
    if (next) {
      localStorage.setItem(TOKEN_STORAGE_KEY, next)
    } else {
      localStorage.removeItem(TOKEN_STORAGE_KEY)
    }
  }, [])

  useEffect(() => {
    if (token) {
      return
    }
    const cookieToken = getCookieAuthToken()
    if (cookieToken) {
      setSessionToken(cookieToken)
    }
  }, [token, setSessionToken])

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
      setSharesLoading(true)
      try {
        const response = await fetch('/api/share', {
          headers: withAuthHeaders(authToken),
        })

        if (response.status === 401) {
          setSessionToken('')
          setShares([])
          setMessage('会话已失效，请在 Control Panel 重新登录。')
          return
        }

        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          setMessage(payload.error ?? `Failed to load shares (${response.status})`)
          return
        }

        const payload = (await response.json()) as { items?: ShareItem[] }
        setShares(Array.isArray(payload.items) ? payload.items : [])
      } finally {
        setSharesLoading(false)
      }
    },
    [setSessionToken],
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

  useEffect(() => {
    if (publicShareId || mainTab !== 'files') {
      return
    }
    void loadDirectory(currentPath, effectiveToken)
  }, [effectiveToken, currentPath, loadDirectory, mainTab, publicShareId])

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
    void loadShares(effectiveToken)
  }, [effectiveToken, loadShares, clearPreviewState, clearSearchState])

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

  const visibleItems = searchActive ? searchResults : items

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

  const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

  const createUploadSession = async (
    authToken: string,
    targetPath: string,
    size: number,
    chunkSize: number,
  ) => {
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
  }

  const getUploadSession = async (authToken: string, sessionId: string) => {
    const response = await fetch(`/api/upload/session/${encodeURIComponent(sessionId)}`, {
      headers: withAuthHeaders(authToken),
    })
    if (!response.ok) {
      return null
    }
    const payload = (await response.json()) as { session?: UploadSessionRecord }
    return payload.session ?? null
  }

  const uploadSessionChunk = async (
    authToken: string,
    sessionId: string,
    offset: number,
    chunk: Blob,
  ) => {
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
  }

  const completeUploadSession = async (authToken: string, sessionId: string) => {
    const response = await fetch(`/api/upload/session/${encodeURIComponent(sessionId)}/complete`, {
      method: 'POST',
      headers: withAuthHeaders(authToken),
    })
    if (!response.ok) {
      const payload = (await response.json().catch(() => ({}))) as { error?: string }
      throw new Error(payload.error ?? `Complete upload failed (${response.status})`)
    }
  }

  const deleteUploadSession = async (authToken: string, sessionId: string) => {
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
  }

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

  const onPreviewFile = (entry: FileEntry) => {
    if (entry.is_dir) {
      return
    }
    clearSearchState()
    setCurrentPath(normalizeUrlPath(entry.path))
  }

  const onClosePreview = useCallback(() => {
    if (!currentPathIsDir) {
      openDirectory(parentPath(currentPath))
      return
    }
    clearPreviewState()
  }, [clearPreviewState, currentPath, currentPathIsDir, openDirectory])

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

  const onDelete = async (entry: FileEntry) => {
    const confirmed = window.confirm(`Delete ${entry.name}?`)
    if (!confirmed) {
      return
    }

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
      await loadDirectory(currentPath, effectiveToken)
      setMessage('Item deleted.')
    } finally {
      setLoading(false)
    }
  }

  const onCreateShare = async (entry: FileEntry) => {
    const expiresInput = window.prompt('Share expiration in seconds (empty for no expiration)', '86400')
    if (expiresInput === null) {
      return
    }
    const expiresRaw = expiresInput.trim()

    let expiresInSeconds: number | undefined
    if (expiresRaw) {
      const parsed = Number(expiresRaw)
      if (!Number.isFinite(parsed) || parsed <= 0) {
        setMessage('Invalid expiration value.')
        return
      }
      expiresInSeconds = Math.floor(parsed)
    }

    const password = window.prompt('Share password (optional)')?.trim() ?? ''

    setLoading(true)
    try {
      const response = await fetch('/api/share', {
        method: 'POST',
        headers: withAuthHeaders(effectiveToken, {
          'Content-Type': 'application/json',
        }),
        body: JSON.stringify({
          path: entry.path,
          password: password || undefined,
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
        setMessage(payload.error ?? `Create share failed (${response.status})`)
        return
      }

      await loadShares(effectiveToken)
      setMessage(`Share created for ${entry.name}`)
    } finally {
      setLoading(false)
    }
  }

  const onDeleteShare = async (shareId: string) => {
    const confirmed = window.confirm('Delete this share link?')
    if (!confirmed) {
      return
    }

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

  const onMoveOrCopy = async (entry: FileEntry, action: 'move' | 'copy') => {
    const suggested = action === 'copy' ? `${entry.path}.copy` : entry.path
    const destination = window.prompt(`${action === 'move' ? 'Move' : 'Copy'} destination path`, suggested)?.trim()
    if (!destination || destination === entry.path) {
      return
    }

    const submit = async (overrideExisting: boolean) =>
      patchResource(entry.path, {
        action,
        destination,
        override_existing: overrideExisting,
      })

    setLoading(true)
    try {
      let response = await submit(false)
      if (response.status === 409) {
        const shouldOverride = window.confirm('Target exists. Override it?')
        if (!shouldOverride) {
          setMessage('Operation cancelled.')
          return
        }
        response = await submit(true)
      }

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `${action} failed (${response.status})`)
        return
      }

      if (action === 'move' && editorPath === entry.path) {
        setEditorPath(destination)
      }
      setSelectedPaths((prev) => prev.filter((item) => item !== entry.path))

      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      setMessage(action === 'move' ? 'Item moved.' : 'Item copied.')
    } finally {
      setLoading(false)
    }
  }

  const onBatchDelete = async () => {
    if (selectedEntries.length === 0) {
      return
    }

    const confirmed = window.confirm(`Delete ${selectedEntries.length} selected item(s)?`)
    if (!confirmed) {
      return
    }

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
      setMessage(`Deleted ${selectedEntries.length} item(s).`)
    } finally {
      setLoading(false)
    }
  }

  const onBatchMoveOrCopy = async (action: 'move' | 'copy') => {
    if (selectedEntries.length === 0) {
      return
    }

    const destinationDir = window
      .prompt(`${action === 'move' ? 'Move' : 'Copy'} destination directory`, currentPath)
      ?.trim()
    if (!destinationDir) {
      return
    }

    const overrideExisting = window.confirm('Override existing targets if conflicts occur?')
    setLoading(true)
    try {
      for (const entry of selectedEntries) {
        const destination = joinPath(destinationDir, fileNameFromPath(entry.path))
        const response = await patchResource(entry.path, {
          action,
          destination,
          override_existing: overrideExisting,
        })
        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          setMessage(payload.error ?? `Batch ${action} failed at ${entry.name}`)
          return
        }

        if (action === 'move' && editorPath === entry.path) {
          setEditorPath(destination)
        }
      }

      setSelectedPaths([])
      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      setMessage(`${action === 'move' ? 'Moved' : 'Copied'} ${selectedEntries.length} item(s).`)
    } finally {
      setLoading(false)
    }
  }

  const onOpenEditor = async (entry: FileEntry) => {
    if (entry.is_dir) {
      return
    }

    setLoading(true)
    try {
      const response = await fetch(`/api/resources${encodePath(entry.path)}`, {
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

  const onCloseEditor = () => {
    if (editorDirty) {
      const confirmed = window.confirm('Discard unsaved changes?')
      if (!confirmed) {
        return
      }
    }
    setEditorPath('')
    setEditorContent('')
    setEditorDirty(false)
    navigateToMainTab('files')
  }

  const onCreateFolder = async () => {
    const folderName = window.prompt('Folder name')?.trim()
    if (!folderName) {
      return
    }

    setLoading(true)
    try {
      const targetPath = `${joinPath(currentPath, folderName)}/`
      const response = await fetch(`/api/resources${encodePath(targetPath)}`, {
        method: 'POST',
        headers: withAuthHeaders(effectiveToken),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `Create folder failed (${response.status})`)
        return
      }

      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      setMessage('Folder created.')
    } finally {
      setLoading(false)
    }
  }

  const onRename = async (entry: FileEntry) => {
    const newName = window.prompt('New name', entry.name)?.trim()
    if (!newName || newName === entry.name) {
      return
    }

    setLoading(true)
    try {
      const response = await fetch(`/api/resources${encodePath(entry.path)}`, {
        method: 'PATCH',
        headers: withAuthHeaders(effectiveToken, {
          'Content-Type': 'application/json',
        }),
        body: JSON.stringify({
          action: 'rename',
          new_name: newName,
        }),
      })

      if (!response.ok) {
        const payload = (await response.json().catch(() => ({}))) as { error?: string }
        setMessage(payload.error ?? `Rename failed (${response.status})`)
        return
      }

      if (editorPath === entry.path) {
        setEditorPath(renamePath(entry.path, newName))
      }

      clearSearchState()
      await loadDirectory(currentPath, effectiveToken)
      setMessage('Item renamed.')
    } finally {
      setLoading(false)
    }
  }

  const downloadQuery = useMemo(() => encodeURIComponent(effectiveToken), [effectiveToken])
  const publicPathSegments = useMemo(() => publicSharePath.split('/').filter(Boolean), [publicSharePath])
  const activeUploadCount = useMemo(
    () => uploadProgress.filter((item) => item.status === 'uploading' || item.status === 'paused').length,
    [uploadProgress],
  )
  const rowActionItemClass =
    'flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-semibold text-slate-700 transition hover:bg-slate-50 hover:text-primary'
  const openActionEntry = useMemo(
    () => visibleItems.find((entry) => entry.path === openActionPath) ?? null,
    [visibleItems, openActionPath],
  )

  const toggleRowActionMenu = useCallback(
    (entryPath: string, event: ReactMouseEvent<HTMLButtonElement>) => {
      if (openActionPath === entryPath) {
        closeRowActionMenu()
        return
      }

      if (typeof window === 'undefined') {
        return
      }

      const triggerRect = event.currentTarget.getBoundingClientRect()
      const viewportWidth = window.innerWidth
      const viewportHeight = window.innerHeight

      let left = triggerRect.right - ROW_ACTION_MENU_WIDTH
      left = Math.max(
        ROW_ACTION_MENU_VIEWPORT_PADDING,
        Math.min(left, viewportWidth - ROW_ACTION_MENU_WIDTH - ROW_ACTION_MENU_VIEWPORT_PADDING),
      )

      let top = triggerRect.bottom + ROW_ACTION_MENU_GAP
      if (top + ROW_ACTION_MENU_ESTIMATED_HEIGHT > viewportHeight - ROW_ACTION_MENU_VIEWPORT_PADDING) {
        top = Math.max(
          ROW_ACTION_MENU_VIEWPORT_PADDING,
          triggerRect.top - ROW_ACTION_MENU_ESTIMATED_HEIGHT - ROW_ACTION_MENU_GAP,
        )
      }

      setActionMenuPosition({ top, left })
      setOpenActionPath(entryPath)
    },
    [openActionPath, closeRowActionMenu],
  )

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

  const uploadQueueCount = uploadProgress.filter((item) => item.status !== 'completed').length
  const previewRawUrl = useMemo(() => {
    if (!previewEntry) {
      return ''
    }
    return `/api/raw${encodePath(previewEntry.path)}?auth=${downloadQuery}`
  }, [downloadQuery, previewEntry])
  const publicImageSrc = useMemo(() => {
    if (!publicShareId) {
      return ''
    }
    return buildPublicDownloadPathForTarget(publicShareId, publicSharePath, publicSharePassword)
  }, [publicShareId, publicSharePassword, publicSharePath])
  const officePreviewUrl = useMemo(() => {
    if (!previewEntry || previewKind !== 'office' || !previewRawUrl) {
      return ''
    }
    return `https://view.officeapps.live.com/op/embed.aspx?src=${encodeURIComponent(`${window.location.origin}${previewRawUrl}`)}`
  }, [previewEntry, previewKind, previewRawUrl])
  const publicTextContent = useMemo(() => {
    if (!publicShareData || publicShareData.is_dir || publicShareData.content == null) {
      return ''
    }
    return publicShareData.content
  }, [publicShareData])
  const publicShareIsImage = useMemo(() => {
    if (!publicShareData || publicShareData.is_dir) {
      return false
    }
    const path = publicSharePath.toLowerCase()
    return (
      path.endsWith('.png') ||
      path.endsWith('.jpg') ||
      path.endsWith('.jpeg') ||
      path.endsWith('.gif') ||
      path.endsWith('.webp') ||
      path.endsWith('.bmp') ||
      path.endsWith('.svg')
    )
  }, [publicShareData, publicSharePath])
  const publicPreviewIsTruncated = publicTextContent.length > PUBLIC_TEXT_PREVIEW_LIMIT
  const publicPreviewContent =
    publicPreviewIsTruncated && !publicPreviewExpanded
      ? `${publicTextContent.slice(0, PUBLIC_TEXT_PREVIEW_LIMIT)}\n\n... (preview truncated)`
      : publicTextContent
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
  const currentPathSegments = useMemo(() => currentPath.split('/').filter(Boolean), [currentPath])
  const visibleFolderCount = useMemo(() => visibleItems.filter((item) => item.is_dir).length, [visibleItems])
  const visibleFileCount = useMemo(() => visibleItems.filter((item) => !item.is_dir).length, [visibleItems])

  useEffect(() => {
    if (previewEntry && previewKind === 'image' && previewRawUrl) {
      setPreviewImageLoading(true)
    } else {
      setPreviewImageLoading(false)
    }
  }, [previewEntry, previewKind, previewRawUrl])

  useEffect(() => {
    if (publicShareData && !publicShareData.is_dir && publicShareIsImage && publicImageSrc) {
      setPublicImageLoading(true)
    } else {
      setPublicImageLoading(false)
    }
  }, [publicImageSrc, publicShareData, publicShareIsImage])

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
                        (publicShareData.items ?? []).map((item) => (
                          <tr key={item.path} className="border-t border-slate-100 text-slate-800">
                            <td className="px-3 py-2 font-medium">
                              {item.is_dir ? (
                                <button
                                  type="button"
                                  onClick={() => void loadPublicShare(publicShareId, publicSharePassword, item.path)}
                                  className="rounded px-1 py-0.5 text-left text-primary transition hover:bg-primary/10"
                                >
                                  <span className="inline-flex items-center gap-1.5">
                                    <Icon name="folder" />
                                    {item.name}
                                  </span>
                                </button>
                              ) : (
                                item.name
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
                                {!item.is_dir ? (
                                  <a
                                    href={buildPublicDownloadPathForTarget(publicShareId, item.path, publicSharePassword)}
                                    className="rounded border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                                  >
                                    <span className="inline-flex items-center gap-1.5">
                                      <Icon name="download" />
                                      Download
                                    </span>
                                  </a>
                                ) : null}
                              </div>
                            </td>
                          </tr>
                        ))
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
                      <pre className="max-h-[420px] overflow-auto rounded-xl border border-slate-200 bg-slate-50 p-3 text-xs text-slate-800">
                        {publicPreviewContent}
                      </pre>
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
                              publicImageSrc,
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
                      <img
                        src={publicImageSrc}
                        alt={publicSharePath}
                        className={`mx-auto max-h-[520px] w-auto max-w-full transition-opacity ${publicImageLoading ? 'opacity-0' : 'opacity-100'}`}
                        loading="lazy"
                        onLoad={() => setPublicImageLoading(false)}
                        onError={() => setPublicImageLoading(false)}
                        onClick={() =>
                          openImageViewer(
                            publicImageSrc,
                            publicSharePath,
                          )
                        }
                      />
                      {publicImageLoading ? (
                        <div className="flex items-center justify-center py-16 text-sm font-medium text-slate-500">Loading image preview...</div>
                      ) : null}
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

        {imageViewerOpen ? (
          <div className={`${embedded ? 'absolute' : 'fixed'} inset-0 z-50 flex flex-col bg-black/85 p-4`}>
            <div className="mx-auto flex w-full max-w-6xl items-center justify-between gap-2 rounded-xl bg-white/95 px-3 py-2">
              <p className="truncate text-sm font-semibold text-slate-800">{imageViewerTitle || 'Image preview'}</p>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={zoomOutImage}
                  className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="zoom-out" />
                    Zoom out
                  </span>
                </button>
                <button
                  type="button"
                  onClick={resetImageZoom}
                  className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="zoom-reset" />
                    Reset
                  </span>
                </button>
                <button
                  type="button"
                  onClick={zoomInImage}
                  className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="zoom-in" />
                    Zoom in
                  </span>
                </button>
                <button
                  type="button"
                  onClick={closeImageViewer}
                  className="rounded-lg border border-rose-300 bg-white px-2 py-1 text-xs font-semibold text-rose-700 transition hover:bg-rose-50"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="close" />
                    Close
                  </span>
                </button>
              </div>
            </div>

            <div className="relative mt-3 flex min-h-0 flex-1 items-center justify-center overflow-auto">
              {viewerImageLoading ? (
                <div className="absolute inset-0 z-10 flex items-center justify-center">
                  <div className="rounded-xl bg-white/95 px-4 py-2 text-sm font-semibold text-slate-700 shadow">Loading image...</div>
                </div>
              ) : null}
              <img
                src={imageViewerSrc}
                alt={imageViewerTitle}
                className={`max-h-full w-auto max-w-none cursor-zoom-in transition-opacity ${viewerImageLoading ? 'opacity-0' : 'opacity-100'}`}
                style={{ transform: `scale(${imageViewerScale})`, transformOrigin: 'center center' }}
                onLoad={() => setViewerImageLoading(false)}
                onError={() => setViewerImageLoading(false)}
                onClick={() => {
                  if (imageViewerScale < 2) {
                    zoomInImage()
                  } else {
                    resetImageZoom()
                  }
                }}
              />
            </div>
          </div>
        ) : null}
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
            <div className="-mb-px flex items-center gap-6">
              <button
                type="button"
                onClick={() => navigateToMainTab('files')}
                className={`border-b-2 px-1 py-3 text-sm font-semibold transition ${
                  mainTab === 'files'
                    ? 'border-primary text-primary'
                    : 'border-transparent text-slate-500 hover:text-primary'
                }`}
              >
                Files
              </button>
              <button
                type="button"
                onClick={() => navigateToMainTab('shares')}
                className={`border-b-2 px-1 py-3 text-sm font-semibold transition ${
                  mainTab === 'shares'
                    ? 'border-primary text-primary'
                    : 'border-transparent text-slate-500 hover:text-primary'
                }`}
              >
                Shares
              </button>
            </div>
          </div>

          {mainTab === 'files' ? (
            <div className={embedded ? 'flex min-h-0 flex-1 flex-col' : ''}>
              <header className="border-b border-slate-200 px-5 py-4">
                <div className="flex flex-wrap items-center gap-2">
                  <label className="inline-flex cursor-pointer items-center rounded-xl bg-primary px-3 py-2 text-sm font-semibold text-white transition hover:bg-teal-700">
                    <span className="inline-flex items-center gap-1.5">
                      <Icon name="upload" />
                      Upload
                    </span>
                    <input type="file" multiple onChange={onUpload} className="hidden" />
                  </label>
                  <button
                    type="button"
                    onClick={() => void onCreateFolder()}
                    className="rounded-xl border border-slate-300 px-3 py-2 text-sm font-medium text-slate-700 transition hover:border-primary hover:text-primary"
                  >
                    <span className="inline-flex items-center gap-1.5">
                      <Icon name="new-folder" />
                      Add folder
                    </span>
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      clearSearchState()
                      void loadDirectory(currentPath, effectiveToken)
                    }}
                    className="rounded-xl border border-slate-300 px-3 py-2 text-sm font-medium text-slate-700 transition hover:border-primary hover:text-primary"
                  >
                    <span className="inline-flex items-center gap-1.5">
                      <Icon name="retry" />
                      Refresh
                    </span>
                  </button>
                  <input
                    value={searchKeyword}
                    onChange={(event) => setSearchKeyword(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') {
                        event.preventDefault()
                        void onSearch()
                      }
                    }}
                    placeholder="Search by file name or path"
                    className="min-w-[240px] flex-1 rounded-xl border border-slate-300 bg-slate-50 px-3 py-2 text-sm text-slate-700 outline-none transition focus:border-primary focus:ring-2 focus:ring-primary/20"
                  />
                  <button
                    type="button"
                    onClick={() => void onSearch()}
                    disabled={searchLoading}
                    className="rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-white transition hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-60"
                  >
                    <span className="inline-flex items-center gap-1.5">
                      <Icon name="search" />
                      {searchLoading ? 'Searching...' : 'Search'}
                    </span>
                  </button>
                  {searchActive ? (
                    <button
                      type="button"
                      onClick={onClearSearch}
                      className="rounded-xl border border-slate-300 px-3 py-2 text-sm font-medium text-slate-700 transition hover:border-primary hover:text-primary"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="clear" />
                        Clear
                      </span>
                    </button>
                  ) : null}
                </div>
              </header>

              <div className="border-b border-slate-200 px-5 py-3">
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0 flex flex-wrap items-center gap-2">
                    <span className="rounded-full bg-slate-100 px-2.5 py-1 text-xs font-semibold text-slate-600">
                      Path: {currentPath}
                    </span>
                    {currentPathIsDir ? (
                      <span className="rounded-full bg-slate-100 px-2.5 py-1 text-xs font-semibold text-slate-600">
                        {visibleFolderCount} folders · {visibleFileCount} files
                      </span>
                    ) : (
                      <span className="rounded-full bg-slate-100 px-2.5 py-1 text-xs font-semibold text-slate-600">File preview mode</span>
                    )}
                    {loading ? <span className="rounded-full bg-slate-100 px-2.5 py-1 text-xs font-semibold text-slate-600">Working...</span> : null}
                    {searchActive ? (
                      <span className="rounded-full bg-amber-50 px-2.5 py-1 text-xs font-semibold text-amber-700">
                        {searchResults.length} result(s){searchTruncated ? ' · truncated' : ''}
                      </span>
                    ) : null}
                    {currentPathIsDir && selectedEntries.length > 0 ? (
                      <span className="rounded-full bg-primary/10 px-2.5 py-1 text-xs font-semibold text-primary">
                        {selectedEntries.length} selected
                      </span>
                    ) : null}
                  </div>

                  {currentPathIsDir ? (
                    <div className="ml-3 flex shrink-0 items-center justify-end gap-2">
                    <button
                      type="button"
                      disabled={selectedEntries.length === 0}
                      onClick={() => void onBatchMoveOrCopy('move')}
                      className="rounded-lg border border-slate-300 px-3 py-2 text-sm font-medium text-slate-700 transition hover:border-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:border-slate-300 disabled:hover:text-slate-700"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="move" />
                        Move selected
                      </span>
                    </button>
                    <button
                      type="button"
                      disabled={selectedEntries.length === 0}
                      onClick={() => void onBatchMoveOrCopy('copy')}
                      className="rounded-lg border border-slate-300 px-3 py-2 text-sm font-medium text-slate-700 transition hover:border-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:border-slate-300 disabled:hover:text-slate-700"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="copy" />
                        Copy selected
                      </span>
                    </button>
                    <button
                      type="button"
                      disabled={selectedEntries.length === 0}
                      onClick={() => void onBatchDelete()}
                      className="rounded-lg border border-rose-300 px-3 py-2 text-sm font-medium text-rose-600 transition hover:bg-rose-50 disabled:cursor-not-allowed disabled:opacity-45 disabled:hover:bg-transparent"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="delete" />
                        Delete selected
                      </span>
                    </button>
                  </div>
                ) : null}
                </div>
              </div>

          {message ? (
            <p className="border-b border-slate-200 bg-slate-50 px-5 py-3 text-sm font-medium text-slate-700">{message}</p>
          ) : null}

          <div className="border-b border-slate-200 bg-white px-5 py-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex flex-wrap items-center gap-1.5">
                <button
                  type="button"
                  onClick={() => openDirectory('/')}
                  className="rounded-md border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="folder" />
                    Root
                  </span>
                </button>
                {currentPathSegments.map((segment, index) => {
                  const partialPath = `/${currentPathSegments.slice(0, index + 1).join('/')}`
                  return (
                    <button
                      key={partialPath}
                      type="button"
                      onClick={() => openDirectory(partialPath)}
                      className="rounded-md border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="folder" />
                        {segment}
                      </span>
                    </button>
                  )
                })}
              </div>

              <button
                type="button"
                onClick={() => openDirectory(parentPath(currentPath))}
                className="rounded-md border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
              >
                <span className="inline-flex items-center gap-1.5">
                  <Icon name="up" />
                  Up
                </span>
              </button>
            </div>
          </div>

          {currentPathIsDir ? (
              <div
                className={`relative px-3 py-3 transition md:px-4 ${
                  embedded ? 'min-h-0 flex-1 overflow-auto' : 'overflow-x-auto'
                } ${
                  dropzoneActive ? 'bg-teal-50/70 ring-2 ring-primary/25 ring-inset' : ''
                }`}
                onDragEnter={onListDragEnter}
                onDragOver={onListDragOver}
                onDragLeave={onListDragLeave}
                onDrop={(event) => void onListDrop(event)}
              >
                {dropzoneActive ? (
                  <div className="pointer-events-none absolute inset-3 z-20 flex items-center justify-center rounded-2xl border-2 border-dashed border-primary/60 bg-white/85">
                    <div className="inline-flex items-center gap-2 rounded-xl bg-white px-3 py-2 text-sm font-semibold text-primary shadow-sm">
                      <Icon name="upload" />
                      Drop files to upload
                    </div>
                  </div>
                ) : null}

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
                      {searchActive ? <th className="bg-slate-50 px-3 py-2">Path</th> : null}
                      <th className="bg-slate-50 px-3 py-2">Type</th>
                      <th className="bg-slate-50 px-3 py-2">Size</th>
                      <th className="bg-slate-50 px-3 py-2">Modified</th>
                      <th className="rounded-r-lg bg-slate-50 px-3 py-2">Actions</th>
                    </tr>
                  </thead>
                  <tbody>
                    {visibleItems.length === 0 ? (
                      <tr>
                        <td colSpan={searchActive ? 7 : 6} className="px-3 py-12 text-center text-sm text-slate-500">
                          {searchActive ? 'No search result.' : 'Empty directory.'}
                        </td>
                      </tr>
                    ) : (
                      visibleItems.map((entry) => (
                        <tr key={entry.path} className="text-slate-800">
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
                                onClick={() => openDirectory(entry.path)}
                                className="rounded px-1 py-0.5 text-left text-primary transition hover:bg-primary/10"
                              >
                                <span className="inline-flex items-center gap-1.5">
                                  <Icon name="folder" />
                                  {entry.name}
                                </span>
                              </button>
                            ) : (
                              <button
                                type="button"
                                onClick={() => void onPreviewFile(entry)}
                                className="rounded px-1 py-0.5 text-left text-slate-800 transition hover:bg-slate-100"
                              >
                                {entry.name}
                              </button>
                            )}
                          </td>
                          {searchActive ? <td className="border-b border-slate-100 px-3 py-2 text-slate-600">{parentPath(entry.path)}</td> : null}
                          <td className="border-b border-slate-100 px-3 py-2">{entry.is_dir ? 'Folder' : 'File'}</td>
                          <td className="border-b border-slate-100 px-3 py-2">{entry.is_dir ? '-' : formatBytes(entry.size)}</td>
                          <td className="border-b border-slate-100 px-3 py-2">{formatTimestamp(entry.modified)}</td>
                          <td className="border-b border-slate-100 px-3 py-2">
                            <div className="inline-flex" data-row-actions="true">
                              <button
                                type="button"
                                onClick={(event) => toggleRowActionMenu(entry.path, event)}
                                className="rounded border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                              >
                                <span className="inline-flex items-center gap-1.5">
                                  <Icon name="more" />
                                  Actions
                                </span>
                              </button>
                            </div>
                          </td>
                        </tr>
                      ))
                    )}
                  </tbody>
                </table>
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
                  {!openActionEntry.is_dir ? (
                    <a
                      href={`/api/raw${encodePath(openActionEntry.path)}?auth=${downloadQuery}`}
                      className={rowActionItemClass}
                      onClick={closeRowActionMenu}
                    >
                      <Icon name="download" />
                      Download
                    </a>
                  ) : null}
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
                </div>,
                document.body,
              )
            : null}

          {previewEntry ? (
            <section
              className={`border-t border-slate-200 bg-slate-50/80 px-5 pt-4 ${
                embedded ? 'min-h-0 flex-1 overflow-y-auto pb-10' : 'pb-4'
              }`}
            >
              <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
                <div>
                  <p className="text-sm font-semibold text-slate-800">File preview</p>
                  <p className="text-xs text-slate-500">
                    {previewEntry.name} · {formatBytes(previewEntry.size)} · {formatTimestamp(previewEntry.modified)}
                  </p>
                </div>
                <button
                  type="button"
                  onClick={onClosePreview}
                  className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="close" />
                    Close preview
                  </span>
                </button>
              </div>

              {previewLoading ? (
                <div className="rounded-xl border border-slate-200 bg-white px-3 py-8 text-center text-sm text-slate-500">
                  Loading preview...
                </div>
              ) : previewError ? (
                <div className="rounded-xl border border-rose-200 bg-rose-50 px-3 py-3 text-sm text-rose-700">{previewError}</div>
              ) : previewKind === 'image' ? (
                <div className="space-y-2 rounded-xl border border-slate-200 bg-white p-3">
                  <div className="flex justify-end">
                    <button
                      type="button"
                      onClick={() => openImageViewer(previewRawUrl, previewEntry.name)}
                      className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                    >
                      <span className="inline-flex items-center gap-1.5">
                        <Icon name="open" />
                        View original
                      </span>
                    </button>
                  </div>
                  <img
                    src={previewRawUrl}
                    alt={previewEntry.name}
                    className={`mx-auto max-h-[520px] w-auto max-w-full transition-opacity ${previewImageLoading ? 'opacity-0' : 'opacity-100'}`}
                    loading="lazy"
                    onLoad={() => setPreviewImageLoading(false)}
                    onError={() => setPreviewImageLoading(false)}
                    onClick={() => openImageViewer(previewRawUrl, previewEntry.name)}
                  />
                  {previewImageLoading ? (
                    <div className="flex items-center justify-center py-16 text-sm font-medium text-slate-500">Loading image preview...</div>
                  ) : null}
                </div>
              ) : previewKind === 'pdf' ? (
                <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
                  <iframe src={previewRawUrl} title={previewEntry.name} className="h-[560px] w-full" />
                </div>
              ) : previewKind === 'text' ? (
                <pre className="max-h-[520px] overflow-auto rounded-xl border border-slate-200 bg-white p-3 text-xs text-slate-800">
                  {previewTextContent || '(empty file)'}
                </pre>
              ) : previewKind === 'office' && officePreviewUrl ? (
                <div className="space-y-2">
                  <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
                    <iframe src={officePreviewUrl} title={`${previewEntry.name} (office preview)`} className="h-[560px] w-full" />
                  </div>
                  <p className="text-xs text-slate-500">If the office preview fails, use download and open locally.</p>
                </div>
              ) : (
                <div className="rounded-xl border border-dashed border-slate-300 bg-white px-3 py-4 text-sm text-slate-600">
                  This file type is not supported for inline preview yet. Use download instead.
                </div>
              )}
            </section>
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

        {imageViewerOpen ? (
          <div className={`${embedded ? 'absolute' : 'fixed'} inset-0 z-50 flex flex-col bg-black/85 p-4`}>
            <div className="mx-auto flex w-full max-w-6xl items-center justify-between gap-2 rounded-xl bg-white/95 px-3 py-2">
              <p className="truncate text-sm font-semibold text-slate-800">{imageViewerTitle || 'Image preview'}</p>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={zoomOutImage}
                  className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="zoom-out" />
                    Zoom out
                  </span>
                </button>
                <button
                  type="button"
                  onClick={resetImageZoom}
                  className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="zoom-reset" />
                    Reset
                  </span>
                </button>
                <button
                  type="button"
                  onClick={zoomInImage}
                  className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="zoom-in" />
                    Zoom in
                  </span>
                </button>
                <button
                  type="button"
                  onClick={closeImageViewer}
                  className="rounded-lg border border-rose-300 bg-white px-2 py-1 text-xs font-semibold text-rose-700 transition hover:bg-rose-50"
                >
                  <span className="inline-flex items-center gap-1.5">
                    <Icon name="close" />
                    Close
                  </span>
                </button>
              </div>
            </div>

            <div className="relative mt-3 flex min-h-0 flex-1 items-center justify-center overflow-auto">
              {viewerImageLoading ? (
                <div className="absolute inset-0 z-10 flex items-center justify-center">
                  <div className="rounded-xl bg-white/95 px-4 py-2 text-sm font-semibold text-slate-700 shadow">Loading image...</div>
                </div>
              ) : null}
              <img
                src={imageViewerSrc}
                alt={imageViewerTitle}
                className={`max-h-full w-auto max-w-none cursor-zoom-in transition-opacity ${viewerImageLoading ? 'opacity-0' : 'opacity-100'}`}
                style={{ transform: `scale(${imageViewerScale})`, transformOrigin: 'center center' }}
                onLoad={() => setViewerImageLoading(false)}
                onError={() => setViewerImageLoading(false)}
                onClick={() => {
                  if (imageViewerScale < 2) {
                    zoomInImage()
                  } else {
                    resetImageZoom()
                  }
                }}
              />
            </div>
          </div>
        ) : null}
      </div>
    </main>
  )
}

export default FileManagerPage
