import { useSyncExternalStore } from 'react'
import { buckyos } from 'buckyos'
import { PreviewEmbedHost, originInZone, type PreviewEmbedInit } from 'buckyos/app-frame'
import { isTransferableRef } from 'buckyos/content'
import { ContentPreview } from '../../../components/ContentPreview'
import type { ContentRef } from '../../../components/preview/types'
import { I18nProvider } from '../../../i18n/provider'
import { PrototypeThemeProvider } from '../../../theme/provider'
import { supportedLocales, type SupportedLocale } from '../../../models/ui'
import { isMockRuntime } from '../../../runtime'

function acceptEmbedder(origin: string): boolean {
  if (isMockRuntime()) return /^https?:\/\/([a-z0-9-]+\.)*(localhost|127\.0\.0\.1)(:\d+)?$/.test(origin)
  const zone = buckyos.getZoneHostName()
  return !!zone && originInZone(origin, zone)
}

function toLocale(locale: string): SupportedLocale {
  const exact = supportedLocales.find(l => l.toLowerCase() === locale.toLowerCase())
  return exact ?? supportedLocales.find(l => l.split('-')[0] === locale.split('-')[0].toLowerCase()) ?? 'en'
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
 * hands over a reference (or, for a draft, the bytes it already holds).
 */
export function PreviewEmbedApp() {
  const { init, theme, locale } = useSyncExternalStore(subscribe, () => view)
  if (!host.parentOrigin) return <p role="alert" className="p-6 text-sm">This page only shows content inside an App.</p>
  if (!init) return null
  const source: ContentRef = init.source.kind === 'blob' ? { kind: 'blob', value: { blob: init.source.blob, name: init.source.name } } : init.source
  const shared = isTransferableRef(init.source) ? init.source : null
  return (
    <PrototypeThemeProvider mode={theme}>
      <I18nProvider locale={toLocale(locale)}>
        <div className="h-[100dvh] w-full" data-testid="preview-embed">
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
          />
        </div>
      </I18nProvider>
    </PrototypeThemeProvider>
  )
}
