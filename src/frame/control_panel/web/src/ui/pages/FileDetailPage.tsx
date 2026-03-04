import { useCallback, useEffect, useMemo, useState } from 'react'
import { ArrowLeft, Download } from 'lucide-react'
import { useLocation } from 'react-router-dom'

import { ensureSessionToken } from '@/auth/authManager'
import { getSessionTokenFromCookies, getStoredSessionToken } from '@/auth/session'
import FilePreviewPanel from '@/ui/components/file_manager/FilePreviewPanel'
import { downloadImageWithProgress } from '@/ui/components/file_manager/imageDownload'
import ImageViewerModal from '@/ui/components/file_manager/ImageViewerModal'
import { getFilePreviewKind, type FilePreviewKind } from '@/ui/components/file_manager/filePreview'

type FileEntry = {
  name: string
  path: string
  is_dir: boolean
  size: number
  modified: number
}

type FileResponse = {
  path: string
  is_dir: boolean
  size: number
  modified: number
  content?: string | null
}

const withAuthHeaders = (authToken: string) => {
  const headers: Record<string, string> = {}
  if (authToken.trim()) {
    headers['X-Auth'] = authToken.trim()
  }
  return headers
}

const encodePath = (path: string) =>
  path
    .split('/')
    .map((segment, index) => (index === 0 ? '' : encodeURIComponent(segment)))
    .join('/')

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

const fileNameFromPath = (path: string) => {
  const parts = path.split('/').filter(Boolean)
  return parts[parts.length - 1] ?? ''
}

const revokeBlobUrl = (url: string) => {
  if (!url.startsWith('blob:')) {
    return
  }
  URL.revokeObjectURL(url)
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

const FileDetailPage = () => {
  const location = useLocation()
  const [token, setToken] = useState(() => getStoredSessionToken() || getSessionTokenFromCookies() || '')
  const [previewEntry, setPreviewEntry] = useState<FileEntry | null>(null)
  const [previewKind, setPreviewKind] = useState<FilePreviewKind>('unknown')
  const [previewTextContent, setPreviewTextContent] = useState('')
  const [previewError, setPreviewError] = useState('')
  const [pageLoading, setPageLoading] = useState(true)
  const [pageError, setPageError] = useState('')
  const [imageViewerOpen, setImageViewerOpen] = useState(false)
  const [imageViewerSrc, setImageViewerSrc] = useState('')
  const [imageViewerTitle, setImageViewerTitle] = useState('')
  const [imageViewerScale, setImageViewerScale] = useState(1)
  const [previewImageLoading, setPreviewImageLoading] = useState(false)
  const [previewImageSrc, setPreviewImageSrc] = useState('')
  const [previewImageLoadedBytes, setPreviewImageLoadedBytes] = useState(0)
  const [previewImageTotalBytes, setPreviewImageTotalBytes] = useState<number | null>(null)
  const [previewImageProgressPercent, setPreviewImageProgressPercent] = useState<number | null>(null)
  const [viewerImageLoading, setViewerImageLoading] = useState(false)

  const effectiveToken = token || getStoredSessionToken() || getSessionTokenFromCookies() || ''
  const requestedPath = useMemo(() => {
    const rawPath = new URLSearchParams(location.search).get('path') || '/'
    return normalizeUrlPath(rawPath)
  }, [location.search])

  const downloadQuery = useMemo(() => encodeURIComponent(effectiveToken), [effectiveToken])
  const buildRawFileUrl = useCallback(
    (path: string, forceDownload = false) =>
      `/api/raw${encodePath(path)}?auth=${downloadQuery}${forceDownload ? '&download=1' : ''}`,
    [downloadQuery],
  )
  const previewRawUrl = useMemo(() => {
    if (!previewEntry) {
      return ''
    }
    return buildRawFileUrl(previewEntry.path)
  }, [buildRawFileUrl, previewEntry])
  const officePreviewUrl = useMemo(() => {
    if (!previewEntry || previewKind !== 'office' || !previewRawUrl) {
      return ''
    }
    return `https://view.officeapps.live.com/op/embed.aspx?src=${encodeURIComponent(`${window.location.origin}${previewRawUrl}`)}`
  }, [previewEntry, previewKind, previewRawUrl])

  const setSessionToken = useCallback((next: string) => {
    setToken(next)
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

  const closeCurrentPage = useCallback(() => {
    if (typeof window === 'undefined') {
      return
    }

    if (window.opener && !window.opener.closed) {
      window.close()
      return
    }

    if (window.history.length > 1) {
      window.history.back()
      return
    }

    window.location.assign('/')
  }, [])

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

    void ensureSessionToken().then((nextToken) => {
      if (!cancelled && nextToken) {
        setSessionToken(nextToken)
      }
    })

    return () => {
      cancelled = true
    }
  }, [token, setSessionToken])

  useEffect(() => {
    let cancelled = false

    const loadFileDetail = async () => {
      setPageLoading(true)
      setPageError('')
      setPreviewError('')
      setPreviewTextContent('')
      setPreviewImageSrc((prev) => {
        revokeBlobUrl(prev)
        return ''
      })
      setPreviewImageLoadedBytes(0)
      setPreviewImageTotalBytes(null)
      setPreviewImageProgressPercent(null)
      setPreviewEntry(null)
      setPreviewKind('unknown')

      if (requestedPath === '/') {
        setPageError('Missing file path. Please reopen from file list.')
        setPageLoading(false)
        return
      }

      if (!effectiveToken) {
        setPageError('Session unavailable. Please log in again from Control Panel.')
        setPageLoading(false)
        return
      }

      try {
        const response = await fetch(`/api/resources${encodePath(requestedPath)}?content=1`, {
          headers: withAuthHeaders(effectiveToken),
        })

        if (cancelled) {
          return
        }

        if (response.status === 401) {
          setSessionToken('')
          setPageError('Session expired. Please log in again from Control Panel.')
          return
        }

        if (!response.ok) {
          const payload = (await response.json().catch(() => ({}))) as { error?: string }
          setPageError(payload.error ?? `Failed to load file (${response.status})`)
          return
        }

        const payload = (await response.json()) as FileResponse
        if (payload.is_dir) {
          setPageError('Target path is a folder. Please choose a file.')
          return
        }

        const normalizedPath = normalizeUrlPath(payload.path || requestedPath)
        const entry: FileEntry = {
          name: fileNameFromPath(normalizedPath),
          path: normalizedPath,
          is_dir: false,
          size: payload.size ?? 0,
          modified: payload.modified ?? 0,
        }
        const kind = getFilePreviewKind(entry)

        setPreviewEntry(entry)
        setPreviewKind(kind)
        if (kind === 'text') {
          if (typeof payload.content === 'string') {
            setPreviewTextContent(payload.content)
          } else {
            setPreviewError('This document preview is unavailable.')
          }
        }
      } finally {
        if (!cancelled) {
          setPageLoading(false)
        }
      }
    }

    void loadFileDetail()
    return () => {
      cancelled = true
    }
  }, [effectiveToken, requestedPath, setSessionToken])

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
    return () => {
      revokeBlobUrl(previewImageSrc)
    }
  }, [previewImageSrc])

  useEffect(() => {
    if (!imageViewerOpen) {
      return
    }

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        closeImageViewer()
        return
      }

      if (event.key === '+' || event.key === '=') {
        event.preventDefault()
        zoomInImage()
        return
      }

      if (event.key === '-') {
        event.preventDefault()
        zoomOutImage()
        return
      }

      if (event.key === '0') {
        event.preventDefault()
        resetImageZoom()
      }
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [closeImageViewer, imageViewerOpen, resetImageZoom, zoomInImage, zoomOutImage])

  return (
    <main className="cp-shell">
      <section className="bucky-file-app cp-panel overflow-hidden">
        <header className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 bg-white px-5 py-4">
          <div>
            <p className="text-sm font-semibold text-slate-800">File detail</p>
            <p className="mt-1 break-all text-xs text-slate-500">{previewEntry?.path || requestedPath}</p>
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={closeCurrentPage}
              className="inline-flex items-center gap-1.5 rounded-lg border border-slate-300 px-3 py-2 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
            >
              <ArrowLeft className="size-4 shrink-0" aria-hidden />
              Back
            </button>
            {previewEntry ? (
              <a
                href={buildRawFileUrl(previewEntry.path, true)}
                className="inline-flex items-center gap-1.5 rounded-lg border border-slate-300 px-3 py-2 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
              >
                <Download className="size-4 shrink-0" aria-hidden />
                Download
              </a>
            ) : null}
          </div>
        </header>

        {pageLoading ? (
          <div className="px-5 py-10 text-center text-sm text-slate-500">Loading file detail...</div>
        ) : pageError ? (
          <div className="px-5 py-4">
            <div className="rounded-xl border border-rose-200 bg-rose-50 px-4 py-3 text-sm text-rose-700">{pageError}</div>
          </div>
        ) : (
          <FilePreviewPanel
            embedded={false}
            previewEntry={previewEntry}
            previewKind={previewKind}
            previewRawUrl={previewRawUrl}
            previewImageSrc={previewImageSrc}
            previewLoading={false}
            previewError={previewError}
            previewTextContent={previewTextContent}
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
        )}
      </section>

      <ImageViewerModal
        open={imageViewerOpen}
        embedded={false}
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
    </main>
  )
}

export default FileDetailPage
