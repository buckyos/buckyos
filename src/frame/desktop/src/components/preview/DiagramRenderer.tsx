import clsx from 'clsx'
import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react'
import { readAll } from './io'
import { DIRECT_SIZE_BUDGET } from './mediaTypes'
import { openDiagram, renderDiagramPage, type DrawioFile } from './drawioSvg'
import type { TranslateFn } from './pinchZoom'
import { isAbortError, PreviewError, type PreviewReadRef } from './types'

interface ShownPage {
  index: number
  readRef: PreviewReadRef
  approximated: boolean
}

export function DiagramRenderer({
  readRef,
  stage,
  textView,
  onRendered,
  onFailure,
  t,
}: {
  readRef: PreviewReadRef
  stage: (page: PreviewReadRef, key: string, onStageRendered: (media?: { width?: number; height?: number }) => void) => ReactNode
  textView?: ReactNode
  onRendered: (media?: { width?: number; height?: number }, opts?: { degraded?: boolean }) => void
  onFailure: (failure: 'corrupted' | 'network') => void
  t: TranslateFn
}) {
  const [file, setFile] = useState<DrawioFile | null>(null)
  const [active, setActive] = useState(0)
  const [shown, setShown] = useState<ShownPage | null>(null)
  const [asText, setAsText] = useState(false)
  const pages = useRef(new Map<number, Promise<ShownPage>>())
  const urls = useRef(new Set<string>())

  useEffect(() => {
    const controller = new AbortController()
    const cache = pages.current
    const owned = urls.current
    void (async () => {
      try {
        const blob = await readAll(readRef, DIRECT_SIZE_BUDGET.diagram, controller.signal)
        const bytes = new Uint8Array(await blob.arrayBuffer())
        let opened: DrawioFile
        try {
          opened = await openDiagram(bytes)
        } catch {
          if (!controller.signal.aborted) onFailure('corrupted')
          return
        }
        if (controller.signal.aborted) return
        setFile(opened)
        if (opened.pages.length === 0) setAsText(true)
      } catch (err) {
        if (controller.signal.aborted || isAbortError(err)) return
        onFailure(err instanceof PreviewError && err.code === 'TOO_LARGE' ? 'corrupted' : 'network')
      }
    })()
    return () => {
      controller.abort()
      cache.clear()
      for (const url of owned) URL.revokeObjectURL(url)
      owned.clear()
    }
  }, [onFailure, readRef])

  const imageText = useCallback(
    (url: string | null) => (url ? t('preview.diagram.imageLink', 'Image: {{url}}', { url }) : t('preview.diagram.image', 'Image')),
    [t],
  )

  useEffect(() => {
    if (!file || file.pages.length === 0) return
    const page = file.pages[active]
    if (!page) return
    let cancelled = false
    let pending = pages.current.get(active)
    if (!pending) {
      pending = renderDiagramPage(page, { imageText }).then((rendered) => {
        const url = URL.createObjectURL(new Blob([rendered.svg], { type: 'image/svg+xml' }))
        urls.current.add(url)
        return { index: active, readRef: { kind: 'url' as const, url }, approximated: rendered.approximated }
      })
      pages.current.set(active, pending)
    }
    pending.then(
      (next) => {
        if (!cancelled) setShown(next)
      },
      () => {
        pages.current.delete(active)
        if (!cancelled) onFailure('corrupted')
      },
    )
    return () => {
      cancelled = true
    }
  }, [active, file, imageText, onFailure])

  const approximated = shown?.approximated ?? false
  const onStageRendered = useCallback(
    (media?: { width?: number; height?: number }) => onRendered(media, { degraded: approximated }),
    [approximated, onRendered],
  )
  useEffect(() => {
    if (file && asText) onRendered(undefined, { degraded: false })
  }, [asText, file, onRendered])

  if (!file) return null
  const showTabs = file.pages.length > 1 || (Boolean(textView) && file.pages.length > 0)
  return (
    <div
      className="absolute inset-0 flex flex-col"
      data-testid="content-preview-diagram"
      data-page-count={file.pages.length}
      data-page={shown?.index ?? ''}
      data-approximated={approximated ? 'true' : 'false'}
    >
      <div className="relative min-h-0 flex-1">
        {asText ? textView : shown && shown.index === active ? stage(shown.readRef, `page-${shown.index}`, onStageRendered) : null}
      </div>
      {showTabs ? (
        <div className="flex h-9 shrink-0 items-stretch border-t border-[color:var(--cp-border)] bg-[color:var(--cp-surface-2-opaque)] text-[12px]">
          <div className="flex min-w-0 flex-1 items-stretch overflow-x-auto" role="tablist">
            {asText
              ? null
              : file.pages.map((page, index) => (
                  <button
                    key={page.id || index}
                    type="button"
                    role="tab"
                    aria-selected={index === active}
                    title={page.name}
                    data-testid="content-preview-diagram-tab"
                    className={clsx(
                      'max-w-[200px] shrink-0 truncate border-r border-[color:var(--cp-border)] px-3',
                      index === active ? 'bg-[color:var(--cp-surface-opaque)] font-medium text-[color:var(--files-link)]' : 'text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]',
                    )}
                    onClick={() => setActive(index)}
                  >
                    {page.name}
                  </button>
                ))}
          </div>
          {textView ? (
            <button
              type="button"
              className="shrink-0 border-l border-[color:var(--cp-border)] px-3 font-medium text-[color:var(--cp-text)] hover:bg-[color:var(--files-hover)]"
              data-testid="content-preview-diagram-text-toggle"
              onClick={() => setAsText((value) => !value)}
            >
              {asText ? t('preview.diagram.asDiagram', 'Diagram') : t('preview.diagram.asSource', 'XML source')}
            </button>
          ) : null}
        </div>
      ) : null}
    </div>
  )
}
