import type { FilePreviewKind } from './filePreview'

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
  previewLoading: boolean
  previewError: string
  previewTextContent: string
  previewImageLoading: boolean
  officePreviewUrl: string
  onClosePreview: () => void
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
  previewLoading,
  previewError,
  previewTextContent,
  previewImageLoading,
  officePreviewUrl,
  onClosePreview,
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
          Close preview
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
              onClick={() => onOpenImageViewer(previewRawUrl, previewEntry.name)}
              className="rounded-lg border border-slate-300 px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
            >
              View original
            </button>
          </div>
          <img
            src={previewRawUrl}
            alt={previewEntry.name}
            className={`mx-auto max-h-[520px] w-auto max-w-full transition-opacity ${previewImageLoading ? 'opacity-0' : 'opacity-100'}`}
            loading="lazy"
            onLoad={onPreviewImageLoad}
            onError={onPreviewImageError}
            onClick={() => onOpenImageViewer(previewRawUrl, previewEntry.name)}
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
