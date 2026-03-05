import type { FilePreviewKind } from './filePreview'
import ProgressRing from './ProgressRing'

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
        <pre className="max-h-[520px] overflow-auto rounded-xl border border-slate-200 bg-white p-3 text-xs text-slate-800">
          {previewTextContent || '(empty file)'}
        </pre>
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
