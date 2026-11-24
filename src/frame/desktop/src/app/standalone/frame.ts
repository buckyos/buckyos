/* ── A system page as the app side of `buckyos.app-frame` ──
 *
 * In the BuckyOS client these pages are the top document of a native window or a mobile container, and the client is
 * their host. In a browser tab there is no host, and the page stands for itself. One connection per document.
 */

import { getBuckyApi, getRuntimeEnvironment } from 'buckyos'
import { AppFrameClient, type AppOpenReply, type FrameInit, type HeaderState } from 'buckyos/app-frame'
import type { ContentHints, OpenRequest } from 'buckyos/content'
import type { AppOpenRequest } from 'buckyos/open-app'
import { matchSupportedLocale, type SupportedLocale, type ThemeMode } from '../../models/ui'
import type { StandalonePageValue } from './context'

export interface FrameHandlers {
  beforeClose: (reason: 'user' | 'logout' | 'shell') => Promise<boolean>
  back: () => boolean
  /** The app edits the account's appearance and is shown in it, not in the client's. */
  ownAppearance: boolean
}

/** How the client shows the page. It is not this page's setting: it is neither stored nor saved to the account. */
export interface FrameAppearance { theme: ThemeMode; locale?: SupportedLocale }

/** The client may say how it shows the page before the handshake, so that the first paint is right. */
function initialPresentation(): StandalonePageValue['presentation'] {
  const env = getBuckyApi()?.env as { presentation?: unknown } | undefined
  return env?.presentation === 'tab' ? 'tab' : 'window'
}

class StandaloneFrame {
  private page: StandalonePageValue = { container: getRuntimeEnvironment().container, presentation: initialPresentation() }
  private client: AppFrameClient | null = null
  private handlers: FrameHandlers | null = null
  private listeners = new Set<() => void>()
  private title: { title: string; dirty: boolean } | null = null
  private appearance: FrameAppearance | null = null

  get inClient(): boolean { return this.page.container !== 'browser' }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  getPage = (): StandalonePageValue => this.page

  getAppearance = (): FrameAppearance | null => (this.handlers?.ownAppearance ? null : this.appearance)

  /** Connects to the client (once) and takes over its requests; returns the function that gives them up. */
  connect(handlers: FrameHandlers): () => void {
    this.handlers = handlers
    if (this.inClient && !this.client) {
      this.client = new AppFrameClient({
        shellOrigin: window.location.origin,
        systemOrigin: window.location.origin,
        onInit: (init) => this.initialized(init),
        onBeforeClose: async (reason) => (await (this.handlers?.beforeClose(reason) ?? true) ? 'allow' : 'deny'),
        onBack: () => this.handlers?.back() ?? false,
        onTheme: (theme) => this.setAppearance(theme.mode, this.appearance?.locale),
        onLocale: (locale) => this.setAppearance(this.appearance?.theme ?? 'light', matchSupportedLocale(locale) ?? undefined),
      })
    }
    return () => { if (this.handlers === handlers) this.handlers = null }
  }

  private initialized(init: FrameInit) {
    const presentation = init.presentation === 'tab' ? 'tab' : 'window'
    const container = init.container ?? this.page.container
    if (presentation !== this.page.presentation || container !== this.page.container) {
      this.page = { container, presentation }
      this.listeners.forEach((listener) => listener())
    }
    this.setAppearance(init.theme.mode, matchSupportedLocale(init.locale) ?? undefined)
    if (this.title) this.client?.setTitle(this.title.title, this.title.dirty)
  }

  private setAppearance(theme: ThemeMode, locale: SupportedLocale | undefined) {
    if (this.appearance?.theme === theme && this.appearance.locale === locale) return
    this.appearance = { theme, locale }
    this.listeners.forEach((listener) => listener())
  }

  setTitle(title: string) {
    this.title = { title, dirty: false }
    this.client?.setTitle(title)
  }

  setHeader(header: HeaderState) {
    this.client?.setHeader(header)
  }

  requestClose() {
    this.client?.requestClose()
  }

  openApp(request: AppOpenRequest): Promise<AppOpenReply> {
    return this.client?.openApp(request) ?? Promise.resolve({ ok: false, code: 'unsupported' })
  }

  /** Hands content to the client to open; false when it does not take it. */
  openContent(request: Omit<OpenRequest, 'requestId'>, options: { newWindow?: boolean; hints?: ContentHints }): boolean {
    if (!this.client?.init?.shellCapabilities.includes('content.open')) return false
    return this.client.openContent(request, 'default', options)
  }
}

export const standaloneFrame = new StandaloneFrame()
