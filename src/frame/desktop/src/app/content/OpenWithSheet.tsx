/* eslint-disable react-refresh/only-export-components */
import { Copy, Download, Import, Loader2, X } from 'lucide-react'
import { useEffect, useState, useSyncExternalStore } from 'react'
import { handlerOfApp, type HandlerPlan, type TransferableContentRef, type TransferableSessionContext } from 'buckyos/content'
import { useI18n } from '../../i18n/provider'
import { ensurePreviewProvider } from '../../components/preview/provider'
import { normalizeCyfsPath } from '../../components/preview/session'
import { extensionRefField, isCyfsPathRef, isObjectIdRef, type ContentRef, type PreviewReadRef, type PreviewSessionContext } from '../../components/preview/types'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import type { PreviewLaunchOrigin } from '../preview/types'
import { contentHandlers, handlerLabel, openContent } from './open'

/** What "Open with…" is asked for: the content, what is known about it and who is asking. Always the original: its
 * name and media type, never those of a Preview conversion (an Office document shown as PDF is still a DOCX here). */
export interface OpenWithTarget {
  source: ContentRef
  session?: PreviewSessionContext
  name: string
  mediaType?: string
  size?: number
  readRef?: PreviewReadRef
  displayName?: string
  /** AppId (or AppInstanceId) of the App asking: its own handlers are not offered back to it. */
  exclude?: string
  /** Offer Preview among the choices (false inside Preview itself). */
  includePreview?: boolean
  origin?: PreviewLaunchOrigin
}

export function OpenWithSheet({ target, onClose, preferFullApp, overlay = 'absolute' }: {
  target: OpenWithTarget
  onClose: () => void
  preferFullApp: boolean
  overlay?: 'absolute' | 'fixed'
}) {
  const { t } = useI18n()
  const [handlers, setHandlers] = useState<HandlerPlan[]>([])
  const [readRef, setReadRef] = useState<PreviewReadRef | undefined>(target.readRef)
  const { source, session, name, mediaType, size, exclude, includePreview } = target
  useEffect(() => {
    let current = true
    void contentHandlers({ source, session }, { name, mime: mediaType, size }).then(plans => {
      if (current) setHandlers(plans.filter(p => (p.appInstanceId ? !exclude || !handlerOfApp(p, exclude) : includePreview)))
    })
    return () => { current = false }
  }, [source, session, name, mediaType, size, exclude, includePreview])
  useEffect(() => {
    if (target.readRef || !(isCyfsPathRef(source) || isObjectIdRef(source))) return
    let current = true
    void ensurePreviewProvider().then(provider => provider.resolvePreviewSource(source, {})).then(resolved => { if (current) setReadRef(resolved.readRef) }).catch(() => {})
    return () => { current = false }
  }, [source, target.readRef])
  // AI Workspace imports the formats its own importers accept (`externalImport.ts`). A Zone file is handed over by its
  // reference and read there with the user's session, as a pasted or dropped one is (剪贴板规范 §7 case 1); other
  // content as the original's bytes. The AI Workspace window imports it on the page the user opens there.
  const fileName = target.displayName ?? name
  const [importable, setImportable] = useState(false)
  const [importing, setImporting] = useState<{ state: 'reading' } | { state: 'failed'; reason: string } | null>(null)
  useEffect(() => {
    let current = true
    // The bytes are handed over in memory: only to a window of this page.
    const hasApp = !desktopUIStore.isStandalone() && desktopUIStore.getSnapshot().apps.some(app => app.id === 'aiworkspace')
    if (hasApp) void import('../aiworkspace/ui/import/externalImport').then(m => { if (current) setImportable(m.importFormatOf(fileName, mediaType) !== null) })
    return () => { current = false }
  }, [fileName, mediaType])
  const importToWorkspace = async () => {
    if (!readRef) return
    setImporting({ state: 'reading' })
    try {
      const { externalImports } = await import('../aiworkspace/ui/import/externalImport')
      const zone = isCyfsPathRef(source) ? { kind: 'cyfs-path' as const, path: normalizeCyfsPath(source.path) }
        : isObjectIdRef(source) ? { kind: 'object-id' as const, objectId: source.objectId, ...(source.innerPath ? { innerPath: source.innerPath } : {}) } : null
      if (zone) {
        externalImports.offer({ name: fileName, items: [{ origin: 'zone', kind: 'file', source: zone, hints: { name: fileName, ...(mediaType ? { mime: mediaType } : {}), ...(size !== undefined ? { size } : {}) } }] })
      } else {
        const blob = readRef.kind === 'blob' ? readRef.blob : await fetch(readRef.url).then(r => { if (!r.ok) throw new Error(`HTTP ${r.status}`); return r.blob() })
        externalImports.offer({ name: fileName, files: [new File([blob], fileName, { type: mediaType ?? blob.type })] })
      }
      desktopUIStore.openAppWindow('aiworkspace')
      onClose()
    } catch (error) {
      setImporting({ state: 'failed', reason: error instanceof Error ? error.message : String(error) })
    }
  }
  const apps = handlers.filter(p => p.appInstanceId)
  const reference = isCyfsPathRef(source) ? normalizeCyfsPath(source.path) : isObjectIdRef(source) ? source.objectId : extensionRefField(source, 'reference') ?? name
  const download = () => {
    if (!readRef) return
    const anchor = document.createElement('a')
    anchor.download = target.displayName ?? name
    if (readRef.kind === 'url') {
      anchor.href = readRef.downloadUrl ?? readRef.url
      anchor.click()
    } else {
      const url = URL.createObjectURL(readRef.blob)
      anchor.href = url
      anchor.click()
      window.setTimeout(() => URL.revokeObjectURL(url), 10_000)
    }
    onClose()
  }
  return (
    <div className={`${overlay} inset-0 z-[1900] flex items-end justify-center bg-[color:color-mix(in_srgb,var(--cp-shadow)_30%,transparent)] p-4 sm:items-center`} onClick={onClose} data-testid="preview-open-with">
      <div
        className="w-full max-w-sm rounded-[20px] border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] p-4 shadow-[var(--cp-panel-shadow)]"
        onClick={(event) => event.stopPropagation()}
        role="dialog"
        aria-label={t('preview.action.openWith', 'Open with…')}
      >
        <div className="flex items-center justify-between">
          <div className="text-[14px] font-semibold">{t('preview.action.openWith', 'Open with…')}</div>
          <button type="button" onClick={onClose} aria-label={t('common.close', 'Close')} className="rounded-full p-1 hover:bg-[color:var(--cp-surface-2)]">
            <X size={14} />
          </button>
        </div>
        <p className="mt-2 text-[12px] leading-5 text-[color:var(--cp-muted)]">
          {!apps.length && preferFullApp
            ? t('previewApp.openWith.noFullApp', 'No dedicated app is installed for {{type}}. You can download the original or copy its reference.', { type: mediaType ?? name })
            : t('previewApp.openWith.fallback', 'Download the original or copy its reference to open it elsewhere.')}
        </p>
        <div className="mt-3 flex flex-col gap-2">
          {handlers.map(plan => <button type="button" key={plan.handlerKey} className="rounded-xl border px-3 py-2 text-left" onClick={() => { void openContent({ source, session, origin: target.origin }, { handlerKey: plan.handlerKey, name, mime: mediaType, size, exclude }); onClose() }}>{handlerLabel(plan)}</button>)}
          {importable && readRef ? (
            <button
              type="button"
              onClick={() => void importToWorkspace()}
              disabled={importing?.state === 'reading'}
              data-testid="preview-open-with-import-aiws"
              className="flex items-center gap-2 rounded-xl border border-[color:var(--cp-border)] px-3 py-2 text-left text-[13px] hover:border-[color:var(--cp-accent)] disabled:opacity-60"
            >
              {importing?.state === 'reading' ? <Loader2 size={15} className="shrink-0 animate-spin" /> : <Import size={15} className="shrink-0" />}
              <span className="min-w-0">
                <span className="block">{importing?.state === 'reading' ? t('previewApp.openWith.importReading', 'Reading {{name}}…', { name: fileName }) : t('previewApp.openWith.importAiws', 'Import to AI Workspace')}</span>
                <span className="block text-[11px] text-[color:var(--cp-muted)]">{t('previewApp.openWith.importAiwsHint', 'Opens AI Workspace; the import starts on the page you open there')}</span>
              </span>
            </button>
          ) : null}
          {importing?.state === 'failed' ? (
            <p role="alert" className="text-[12px] text-[color:var(--cp-danger,#c62828)]" data-testid="preview-open-with-import-error">
              {t('previewApp.openWith.importFailed', 'Could not read {{name}}: {{reason}}', { name: fileName, reason: importing.reason })}
            </p>
          ) : null}
          {readRef ? (
            <button type="button" onClick={download} className="flex items-center gap-2 rounded-xl border border-[color:var(--cp-border)] px-3 py-2 text-[13px] hover:border-[color:var(--cp-accent)]">
              <Download size={15} /> {t('preview.action.download', 'Download original')}
            </button>
          ) : null}
          <button
            type="button"
            onClick={() => {
              void navigator.clipboard?.writeText(reference).catch(() => {})
              onClose()
            }}
            className="flex items-center gap-2 rounded-xl border border-[color:var(--cp-border)] px-3 py-2 text-[13px] hover:border-[color:var(--cp-accent)]"
          >
            <Copy size={15} /> {t('preview.action.copySource', 'Copy path / object id')}
          </button>
        </div>
      </div>
    </div>
  )
}

// The system "Open with…" for requests that do not come from a Preview window: an App (`content.open` with
// target `chooser`) or a deep link (`/?open=…&intent=chooser`). One request at a time; a new one replaces it.
let pendingTarget: OpenWithTarget | null = null
const listeners = new Set<() => void>()
export function requestOpenWith(target: { source: TransferableContentRef; session?: TransferableSessionContext } & Omit<OpenWithTarget, 'source' | 'session'>): void {
  pendingTarget = { ...target, includePreview: target.includePreview ?? true }
  listeners.forEach(fn => fn())
}
const subscribe = (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn) } }

export function OpenWithHost({ preferFullApp = true }: { preferFullApp?: boolean }) {
  const target = useSyncExternalStore(subscribe, () => pendingTarget)
  if (!target) return null
  return <OpenWithSheet key={JSON.stringify([target.source, target.exclude])} target={target} preferFullApp={preferFullApp} overlay="fixed" onClose={() => { pendingTarget = null; listeners.forEach(fn => fn()) }} />
}
