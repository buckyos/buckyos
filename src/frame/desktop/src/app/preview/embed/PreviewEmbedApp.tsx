import { lazy, Suspense, useEffect, useState, useSyncExternalStore } from 'react'
import { buckyos } from 'buckyos'
import { PreviewEmbedHost, originInZone, type PreviewEmbedInit } from 'buckyos/app-frame'
import { isTransferableRef } from 'buckyos/content'
import { ContentPreview } from '../../../components/ContentPreview'
import type { ContentRef } from '../../../components/preview/types'
import { I18nProvider, useI18n } from '../../../i18n/provider'
import type { ShareDialogParams } from '../../../sysdlg/ShareDialog'
import { PrototypeThemeProvider } from '../../../theme/provider'
import { matchSupportedLocale, type SupportedLocale } from '../../../models/ui'
import { isMockRuntime } from '../../../runtime'
import { shareParamsOf } from '../../content/share'

const ShareDialog = lazy(() => import('../../../sysdlg/ShareDialog').then((m) => ({ default: m.ShareDialog })))

function acceptEmbedder(origin: string): boolean {
  if (isMockRuntime()) return /^https?:\/\/([a-z0-9-]+\.)*(localhost|127\.0\.0\.1)(:\d+)?$/.test(origin)
  const zone = buckyos.getZoneHostName()
  return !!zone && originInZone(origin, zone)
}

function toLocale(locale: string): SupportedLocale {
  return matchSupportedLocale(locale) ?? 'en'
}

// One embed per document: the page is created by the App for a single content reference.
let view: { init: PreviewEmbedInit | null; theme: 'light' | 'dark'; locale: string } = { init: null, theme: 'light', locale: 'en' }
const listeners = new Set<() => void>()
const publish = (next: Partial<typeof view>) => { view = { ...view, ...next }; listeners.forEach(fn => fn()) }
const host = new PreviewEmbedHost({
  accept: acceptEmbedder,
  onInit: init => { publish({ init, theme: init.theme, locale: init.locale }); host.state({ status: 'loading' }) },
  onUpdate: patch => publish(patch),
})
const subscribe = (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn) } }

/**
 * The system Preview component alone, for an App to show inside its own UI (`buckyos.preview-embed`): no
 * Desktop shell, no Preview App window. The content is read with this origin's own session; the App only ever
 * hands over a reference (or, for a draft, the bytes it already holds). The toolbar starts shown (the App asked
 * for a viewer) and the user may let it hide; Share opens the system share dialog here, with links only (hand-offs
 * to other apps need the Desktop shell).
 */
export function PreviewEmbedApp() {
  const { init, theme, locale } = useSyncExternalStore(subscribe, () => view)
  if (!host.parentOrigin) return <p role="alert" className="p-6 text-sm">This page only shows content inside an App.</p>
  if (!init) return null
  return (
    <PrototypeThemeProvider mode={theme}>
      <I18nProvider locale={toLocale(locale)}>
        <EmbeddedPreview init={init} />
      </I18nProvider>
    </PrototypeThemeProvider>
  )
}

function EmbeddedPreview({ init }: { init: PreviewEmbedInit }) {
  const { t } = useI18n()
  const [sharing, setSharing] = useState<ShareDialogParams | null>(null)
  const [notice, setNotice] = useState<string | null>(null)
  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(null), 5000)
    return () => window.clearTimeout(timer)
  }, [notice])
  const source: ContentRef = init.source.kind === 'blob' ? { kind: 'blob', value: { blob: init.source.blob, name: init.source.name } } : init.source
  const shared = isTransferableRef(init.source) ? init.source : null
  return (
    <div className="relative h-[100dvh] w-full" data-testid="preview-embed">
      <ContentPreview
        source={source}
        session={init.session}
        uiMode="visible"
        autoFocus
        onItemChanged={() => host.state({ status: 'loading' })}
        onReady={info => host.state({ status: 'ready', title: info.title, mediaType: info.mediaType })}
        onError={error => host.state({ status: 'error', error: { kind: error.kind, code: error.code } })}
        onRequestExit={() => host.exit()}
        onRequestOpenWith={shared ? request => host.openWith({
          request: { source: isTransferableRef(request.item.source) ? request.item.source : shared, session: init.session },
          hints: { name: request.item.title, mime: request.mediaType, size: request.resolved?.size },
        }) : undefined}
        onRequestShare={shared ? request => {
          const params = shareParamsOf(request.item.source, request.item.title)
          if (params) setSharing(params)
          else setNotice(t('previewApp.share.unavailable', 'Only files in your Zone can be shared; {{name}} is not one, so it cannot be shared from here.', { name: request.item.title }))
        } : undefined}
      />
      {sharing ? (
        <div
          className="fixed inset-0 z-[2000] flex items-center justify-center bg-[color:color-mix(in_srgb,var(--cp-shadow,#000)_35%,transparent)] p-3"
          data-testid="preview-embed-share"
          onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); setSharing(null) } }}
        >
          <div role="dialog" aria-modal="true" aria-label={t('share.dialog.title', 'Share')} className="files-ui max-h-full w-full max-w-md overflow-auto rounded-[12px] border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] p-4 shadow-[var(--cp-panel-shadow)]">
            <Suspense fallback={null}>
              <ShareDialog params={sharing} shell={false} onDone={() => setSharing(null)} />
            </Suspense>
            <div className="mt-3 flex justify-end">
              <button type="button" className="files-btn" onClick={() => setSharing(null)} data-testid="preview-embed-share-close">
                {t('common.close', 'Close')}
              </button>
            </div>
          </div>
        </div>
      ) : null}
      {notice ? (
        <div role="status" className="files-ui files-toast pointer-events-none fixed bottom-4 left-1/2 z-[2000] max-w-[90%] -translate-x-1/2" data-testid="preview-embed-notice">
          {notice}
        </div>
      ) : null}
    </div>
  )
}
