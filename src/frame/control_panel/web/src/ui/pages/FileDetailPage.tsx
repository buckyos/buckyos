import { useCallback, useEffect, useMemo, useState } from 'react'
import { ArrowLeft, ChevronLeft, ChevronRight, Download } from 'lucide-react'
import mammoth from 'mammoth/mammoth.browser'
import { useLocation, useNavigate } from 'react-router-dom'

import { ensureSessionToken } from '@/auth/authManager'
import { getSessionTokenFromCookies, getStoredSessionToken } from '@/auth/session'
import FilePreviewPanel from '@/ui/components/file_manager/FilePreviewPanel'
import { downloadImageWithProgress } from '@/ui/components/file_manager/imageDownload'
import ImageViewerModal from '@/ui/components/file_manager/ImageViewerModal'
import { getFilePreviewKind, isDocFileName, type FilePreviewKind } from '@/ui/components/file_manager/filePreview'

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

type FileNavResponse = {
  path: string
  previous?: FileEntry | null
  next?: FileEntry | null
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
  const navigate = useNavigate()
  const location = useLocation()
  const [token, setToken] = useState(() => getStoredSessionToken() || getSessionTokenFromCookies() || '')
  const [previewEntry, setPreviewEntry] = useState<FileEntry | null>(null)
  const [previewKind, setPreviewKind] = useState<FilePreviewKind>('unknown')
  const [previewTextContent, setPreviewTextContent] = useState('')
  const [previewDocxHtml, setPreviewDocxHtml] = useState('')
  const [previewDocPdfSrc, setPreviewDocPdfSrc] = useState('')
  const [previewLoading, setPreviewLoading] = useState(false)
  const [previewLoadingLabel, setPreviewLoadingLabel] = useState('Loading preview...')
  const [previewLoadingElapsedSeconds, setPreviewLoadingElapsedSeconds] = useState(0)
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
  const [navLoading, setNavLoading] = useState(false)
  const [previousEntry, setPreviousEntry] = useState<FileEntry | null>(null)
  const [nextEntry, setNextEntry] = useState<FileEntry | null>(null)

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
  const previewIsDocFile = useMemo(() => {
    if (!previewEntry || previewKind !== 'office') {
      return false
    }
    return isDocFileName(previewEntry.name)
  }, [previewEntry, previewKind])
  const previewDocPdfUrl = useMemo(() => {
    if (!previewEntry || !previewIsDocFile) {
      return ''
    }
    return `/api/preview/pdf${encodePath(previewEntry.path)}?auth=${downloadQuery}`
  }, [downloadQuery, previewEntry, previewIsDocFile])
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

  const openDetailPath = useCallback(
    (path: string) => {
      navigate(`/files/detail?path=${encodeURIComponent(normalizeUrlPath(path))}`)
    },
    [navigate],
  )

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
      setPreviewDocxHtml('')
      setPreviewDocPdfSrc((prev) => {
        revokeBlobUrl(prev)
        return ''
      })
      setPreviewLoading(false)
      setPreviewLoadingLabel('Loading preview...')
      setPreviewLoadingElapsedSeconds(0)
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
        setPreviewLoading(false)
        setPreviewLoadingLabel('Loading preview...')
        setPreviewLoadingElapsedSeconds(0)
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

    const loadFileNav = async () => {
      setPreviousEntry(null)
      setNextEntry(null)

      if (!effectiveToken || requestedPath === '/') {
        return
      }

      setNavLoading(true)
      try {
        const response = await fetch(`/api/resources/nav?path=${encodeURIComponent(requestedPath)}`, {
          headers: withAuthHeaders(effectiveToken),
        })

        if (cancelled) {
          return
        }

        if (response.status === 401) {
          setSessionToken('')
          return
        }

        if (!response.ok) {
          return
        }

        const payload = (await response.json().catch(() => ({}))) as FileNavResponse
        setPreviousEntry(payload.previous ?? null)
        setNextEntry(payload.next ?? null)
      } finally {
        if (!cancelled) {
          setNavLoading(false)
        }
      }
    }

    void loadFileNav()
    return () => {
      cancelled = true
    }
  }, [effectiveToken, requestedPath, setSessionToken])

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
    return () => {
      revokeBlobUrl(previewImageSrc)
    }
  }, [previewImageSrc])

  useEffect(() => {
    return () => {
      revokeBlobUrl(previewDocPdfSrc)
    }
  }, [previewDocPdfSrc])

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
            <button
              type="button"
              onClick={() => {
                if (previousEntry) {
                  openDetailPath(previousEntry.path)
                }
              }}
              disabled={!previousEntry}
              className="inline-flex items-center gap-1.5 rounded-lg border border-slate-300 px-3 py-2 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-50"
            >
              <ChevronLeft className="size-4 shrink-0" aria-hidden />
              Prev
            </button>
            <button
              type="button"
              onClick={() => {
                if (nextEntry) {
                  openDetailPath(nextEntry.path)
                }
              }}
              disabled={!nextEntry}
              className="inline-flex items-center gap-1.5 rounded-lg border border-slate-300 px-3 py-2 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary disabled:cursor-not-allowed disabled:opacity-50"
            >
              Next
              <ChevronRight className="size-4 shrink-0" aria-hidden />
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
            {navLoading ? <span className="text-xs text-slate-400">Loading nav...</span> : null}
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
