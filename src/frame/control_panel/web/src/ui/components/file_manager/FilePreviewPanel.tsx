import { useEffect, useMemo, useState } from 'react'

import { getTextPreviewMode, type FilePreviewKind } from './filePreview'
import { renderMarkdownHtml } from './markdownPreview'
import ProgressRing from './ProgressRing'

const TEXT_PREVIEW_LIMIT = 200_000
const CSV_PREVIEW_MAX_ROWS = 80
const CSV_PREVIEW_MAX_COLS = 20

const parseCsvRows = (content: string, maxRows = CSV_PREVIEW_MAX_ROWS, maxCols = CSV_PREVIEW_MAX_COLS) => {
  const rows: string[][] = []
  let row: string[] = []
  let cell = ''
  let inQuotes = false

  const pushCell = () => {
    row.push(cell)
    cell = ''
  }

  const pushRow = () => {
    pushCell()
    rows.push(row.slice(0, maxCols))
    row = []
  }

  for (let index = 0; index < content.length; index += 1) {
    const char = content[index]
    const next = content[index + 1]

    if (char === '"') {
      if (inQuotes && next === '"') {
        cell += '"'
        index += 1
      } else {
        inQuotes = !inQuotes
      }
      continue
    }

    if (!inQuotes && char === ',') {
      pushCell()
      continue
    }

    if (!inQuotes && (char === '\n' || char === '\r')) {
      if (char === '\r' && next === '\n') {
        index += 1
      }
      pushRow()
      if (rows.length >= maxRows) {
        return rows
      }
      continue
    }

    cell += char
  }

  if (cell.length > 0 || row.length > 0) {
    pushRow()
  }

  return rows.slice(0, maxRows)
}

type PreviewFileEntry = {
  name: string
  size: number
  modified: number
}

type FilePreviewPanelProps = {
  embedded: boolean
  previewEntry: PreviewFileEntry | null
  previewKind: FilePreviewKind
  previewRawUrl: string
  previewImageSrc: string
  previewLoading: boolean
  previewError: string
  previewTextContent: string
  previewDocxHtml: string
  previewImageLoading: boolean
  previewImageProgressPercent: number | null
  previewImageLoadedBytes: number
  previewImageTotalBytes: number | null
  previewLoadingLabel?: string
  previewLoadingElapsedSeconds?: number
  officePreviewUrl: string
  onOpenImageViewer: (src: string, title: string) => void
  onPreviewImageLoad: () => void
  onPreviewImageError: () => void
  formatBytes: (value: number) => string
  formatTimestamp: (value: number) => string
}

const FilePreviewPanel = ({
  embedded,
  previewEntry,
  previewKind,
  previewRawUrl,
  previewImageSrc,
  previewLoading,
  previewError,
  previewTextContent,
  previewDocxHtml,
  previewImageLoading,
  previewImageProgressPercent,
  previewImageLoadedBytes,
  previewImageTotalBytes,
  previewLoadingLabel,
  previewLoadingElapsedSeconds,
  officePreviewUrl,
  onOpenImageViewer,
  onPreviewImageLoad,
  onPreviewImageError,
  formatBytes,
  formatTimestamp,
}: FilePreviewPanelProps) => {
  const [textExpanded, setTextExpanded] = useState(false)

  const textMode = useMemo(() => {
    if (!previewEntry || previewKind !== 'text') {
      return 'plain'
    }
    return getTextPreviewMode(previewEntry.name)
  }, [previewEntry, previewKind])

  const textIsTruncated = previewTextContent.length > TEXT_PREVIEW_LIMIT

  const activeTextContent = useMemo(() => {
    if (!textIsTruncated || textExpanded) {
      return previewTextContent
    }
    return `${previewTextContent.slice(0, TEXT_PREVIEW_LIMIT)}\n\n... (preview truncated)`
  }, [previewTextContent, textExpanded, textIsTruncated])

  const jsonPreviewContent = useMemo(() => {
    if (textMode !== 'json' || !activeTextContent.trim()) {
      return activeTextContent
    }
    try {
      return JSON.stringify(JSON.parse(activeTextContent), null, 2)
    } catch {
      return activeTextContent
    }
  }, [activeTextContent, textMode])

  const csvRows = useMemo(() => {
    if (textMode !== 'csv') {
      return []
    }
    return parseCsvRows(activeTextContent)
  }, [activeTextContent, textMode])

  const markdownHtml = useMemo(() => {
    if (textMode !== 'markdown') {
      return ''
    }
    return renderMarkdownHtml(activeTextContent)
  }, [activeTextContent, textMode])

  useEffect(() => {
    setTextExpanded(false)
  }, [previewEntry?.modified, previewEntry?.name, previewEntry?.size])

  if (!previewEntry) {
    return null
  }

  return (
    <section
      className={`border-t border-slate-200 bg-slate-50/80 px-5 pt-4 ${
        embedded ? 'min-h-0 flex-1 overflow-y-auto pb-14' : 'pb-6'
      }`}
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div>
          <p className="text-sm font-semibold text-slate-800">File preview</p>
          <p className="text-xs text-slate-500">
            {previewEntry.name} · {formatBytes(previewEntry.size)} · {formatTimestamp(previewEntry.modified)}
          </p>
        </div>
      </div>

      {previewLoading ? (
        <div className="rounded-xl border border-slate-200 bg-white px-3 py-8 text-center text-sm text-slate-500">
          <p>{previewLoadingLabel || 'Loading preview...'}</p>
          {previewLoadingElapsedSeconds && previewLoadingElapsedSeconds > 0 ? (
            <p className="mt-1 text-xs text-slate-400">{previewLoadingElapsedSeconds}s elapsed</p>
          ) : null}
        </div>
      ) : previewError ? (
        <div className="rounded-xl border border-rose-200 bg-rose-50 px-3 py-3 text-sm text-rose-700">{previewError}</div>
      ) : previewKind === 'image' ? (
        <div className="space-y-2 rounded-xl border border-slate-200 bg-white px-3 pt-3 pb-10">
          <div className="flex justify-center">
            <button
              type="button"
              onClick={() => onOpenImageViewer(previewImageSrc || previewRawUrl, previewEntry.name)}
              className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
            >
              View original
            </button>
          </div>
          {previewImageSrc ? (
            <img
              src={previewImageSrc}
              alt={previewEntry.name}
              className={`mx-auto max-h-[520px] w-auto max-w-full transition-opacity ${previewImageLoading ? 'opacity-0' : 'opacity-100'}`}
              loading="lazy"
              onLoad={onPreviewImageLoad}
              onError={onPreviewImageError}
              onClick={() => onOpenImageViewer(previewImageSrc || previewRawUrl, previewEntry.name)}
            />
          ) : null}
          {previewImageLoading ? (
            <div className="flex flex-col items-center gap-2 px-1 py-10 text-center">
              <ProgressRing progressPercent={previewImageProgressPercent} />
              <p className="text-xs font-medium text-slate-600">Loading image preview...</p>
              <p className="text-[11px] text-slate-500">
                {previewImageTotalBytes != null
                  ? `${formatBytes(previewImageLoadedBytes)} / ${formatBytes(previewImageTotalBytes)}`
                  : formatBytes(previewImageLoadedBytes)}
              </p>
            </div>
          ) : null}
        </div>
      ) : previewKind === 'pdf' ? (
        <div className="overflow-hidden rounded-xl border border-slate-200 bg-white">
          <iframe src={previewRawUrl} title={previewEntry.name} className="h-[560px] w-full" />
        </div>
      ) : previewKind === 'text' ? (
        <div className="space-y-2">
          {textMode === 'csv' && csvRows.length > 0 ? (
            <div className="overflow-auto rounded-xl border border-slate-200 bg-white">
              <table className="w-full min-w-[480px] text-xs text-slate-700">
                <tbody>
                  {csvRows.map((row, rowIndex) => (
                    <tr key={`csv-${rowIndex}`} className="border-t border-slate-100 first:border-t-0">
                      {row.map((cell, cellIndex) => (
                        <td
                          key={`csv-${rowIndex}-${cellIndex}`}
                          className={`px-2 py-1.5 align-top ${rowIndex === 0 ? 'bg-slate-50 font-semibold text-slate-800' : ''}`}
                        >
                          {cell || (rowIndex === 0 ? `Column ${cellIndex + 1}` : '')}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : textMode === 'markdown' ? (
            markdownHtml.trim() ? (
              <article
                className="max-h-[560px] overflow-auto rounded-xl border border-slate-200 bg-white p-4 text-sm leading-7 text-slate-800 [&_a]:text-primary [&_a]:underline [&_a]:underline-offset-2 [&_blockquote]:my-3 [&_blockquote]:rounded-r-lg [&_blockquote]:border-l-4 [&_blockquote]:border-primary/40 [&_blockquote]:bg-teal-50/40 [&_blockquote]:px-3 [&_blockquote]:py-2 [&_blockquote_code]:rounded [&_blockquote_code]:bg-slate-100 [&_blockquote_code]:px-1 [&_blockquote_code]:py-0.5 [&_blockquote_code]:text-[0.85em] [&_h1]:mt-1 [&_h1]:text-2xl [&_h1]:font-semibold [&_h2]:mt-4 [&_h2]:text-xl [&_h2]:font-semibold [&_h3]:mt-3 [&_h3]:text-lg [&_h3]:font-semibold [&_hr]:my-4 [&_hr]:border-slate-200 [&_li]:my-1 [&_li_code]:rounded [&_li_code]:bg-slate-100 [&_li_code]:px-1 [&_li_code]:py-0.5 [&_li_code]:text-[0.85em] [&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:my-2 [&_p_code]:rounded [&_p_code]:bg-slate-100 [&_p_code]:px-1 [&_p_code]:py-0.5 [&_p_code]:text-[0.85em] [&_pre]:my-3 [&_pre]:overflow-auto [&_pre]:rounded-lg [&_pre]:bg-slate-950 [&_pre]:p-3 [&_pre]:text-xs [&_pre]:text-slate-100 [&_pre_code]:bg-transparent [&_pre_code]:p-0 [&_pre_code]:text-slate-100 [&_strong]:font-semibold [&_table]:my-3 [&_table]:w-full [&_table]:border-collapse [&_tbody_td]:border [&_tbody_td]:border-slate-200 [&_tbody_td]:px-2 [&_tbody_td]:py-1.5 [&_tbody_tr:nth-child(even)]:bg-slate-50 [&_thead_th]:border [&_thead_th]:border-slate-200 [&_thead_th]:bg-slate-100 [&_thead_th]:px-2 [&_thead_th]:py-1.5 [&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5"
                dangerouslySetInnerHTML={{ __html: markdownHtml }}
              />
            ) : (
              <pre className="max-h-[520px] overflow-auto rounded-xl border border-slate-200 bg-white p-3 text-xs text-slate-800">
                (empty file)
              </pre>
            )
          ) : (
            <pre
              className={`max-h-[520px] overflow-auto rounded-xl border border-slate-200 p-3 text-xs ${
                textMode === 'code' ? 'bg-slate-950 text-slate-100' : 'bg-white text-slate-800'
              }`}
            >
              {jsonPreviewContent || '(empty file)'}
            </pre>
          )}

          {textIsTruncated ? (
            <div className="flex items-center justify-between gap-2 text-xs text-slate-500">
              <span>
                Preview shows {formatBytes(activeTextContent.length)} of {formatBytes(previewTextContent.length)} text.
              </span>
              <button
                type="button"
                onClick={() => setTextExpanded((prev) => !prev)}
                className="rounded-lg border border-slate-300 px-2 py-1 font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
              >
                {textExpanded ? 'Show truncated preview' : 'Show full text'}
              </button>
            </div>
          ) : null}
        </div>
      ) : previewKind === 'docx' ? (
        <div className="max-h-[560px] overflow-auto rounded-xl border border-slate-200 bg-white p-4">
          {previewDocxHtml ? (
            <article
              className="prose prose-sm max-w-none text-slate-800"
              dangerouslySetInnerHTML={{ __html: previewDocxHtml }}
            />
          ) : (
            <p className="text-sm text-slate-500">This DOCX file has no previewable content.</p>
          )}
        </div>
      ) : previewKind === 'audio' ? (
        <div className="space-y-3 rounded-xl border border-slate-200 bg-white p-3">
          <audio controls preload="metadata" className="w-full">
            <source src={previewRawUrl} />
            Your browser does not support audio preview.
          </audio>
          <p className="text-xs text-slate-500">Audio preview</p>
        </div>
      ) : previewKind === 'video' ? (
        <div className="space-y-3 rounded-xl border border-slate-200 bg-white p-3">
          <video controls playsInline preload="metadata" className="max-h-[560px] w-full rounded-lg bg-black" src={previewRawUrl}>
            Your browser does not support video preview.
          </video>
          <p className="text-xs text-slate-500">Video preview</p>
        </div>
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
  )
}

export default FilePreviewPanel
