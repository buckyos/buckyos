import { RotateCw, X, ZoomIn, ZoomOut } from 'lucide-react'

type ImageViewerModalProps = {
  open: boolean
  embedded: boolean
  title: string
  src: string
  scale: number
  loading: boolean
  onZoomOut: () => void
  onResetZoom: () => void
  onZoomIn: () => void
  onClose: () => void
  onImageLoad: () => void
  onImageError: () => void
  onImageClick: () => void
}

const ImageViewerModal = ({
  open,
  embedded,
  title,
  src,
  scale,
  loading,
  onZoomOut,
  onResetZoom,
  onZoomIn,
  onClose,
  onImageLoad,
  onImageError,
  onImageClick,
}: ImageViewerModalProps) => {
  if (!open) {
    return null
  }

  return (
    <div className={`${embedded ? 'absolute' : 'fixed'} inset-0 z-50 flex flex-col bg-black/85 p-4`}>
      <div className="mx-auto flex w-full max-w-6xl items-center justify-between gap-2 rounded-xl bg-white/95 px-3 py-2">
        <p className="truncate text-sm font-semibold text-slate-800">{title || 'Image preview'}</p>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={onZoomOut}
            className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
          >
            <span className="inline-flex items-center gap-1.5">
              <ZoomOut className="size-4 shrink-0" aria-hidden />
              Zoom out
            </span>
          </button>
          <button
            type="button"
            onClick={onResetZoom}
            className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
          >
            <span className="inline-flex items-center gap-1.5">
              <RotateCw className="size-4 shrink-0" aria-hidden />
              Reset
            </span>
          </button>
          <button
            type="button"
            onClick={onZoomIn}
            className="rounded-lg border border-slate-300 bg-white px-2 py-1 text-xs font-semibold text-slate-700 transition hover:border-primary hover:text-primary"
          >
            <span className="inline-flex items-center gap-1.5">
              <ZoomIn className="size-4 shrink-0" aria-hidden />
              Zoom in
            </span>
          </button>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-rose-300 bg-white px-2 py-1 text-xs font-semibold text-rose-700 transition hover:bg-rose-50"
          >
            <span className="inline-flex items-center gap-1.5">
              <X className="size-4 shrink-0" aria-hidden />
              Close
            </span>
          </button>
        </div>
      </div>

      <div className="relative mt-3 flex min-h-0 flex-1 items-center justify-center overflow-auto">
        {loading ? (
          <div className="absolute inset-0 z-10 flex items-center justify-center">
            <div className="rounded-xl bg-white/95 px-4 py-2 text-sm font-semibold text-slate-700 shadow">Loading image...</div>
          </div>
        ) : null}
        <img
          src={src}
          alt={title}
          className={`max-h-full w-auto max-w-none cursor-zoom-in transition-opacity ${loading ? 'opacity-0' : 'opacity-100'}`}
          style={{ transform: `scale(${scale})`, transformOrigin: 'center center' }}
          onLoad={onImageLoad}
          onError={onImageError}
          onClick={onImageClick}
        />
      </div>
    </div>
  )
}

export default ImageViewerModal
