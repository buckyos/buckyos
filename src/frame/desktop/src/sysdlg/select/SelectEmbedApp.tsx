import { useEffect, useSyncExternalStore } from 'react'
import { X } from 'lucide-react'
import { buckyos } from 'buckyos'
import { SelectEmbedHost, originInZone, type SelectEmbedInit } from 'buckyos/app-frame'
import { I18nProvider, useI18n } from '../../i18n/provider'
import { PrototypeThemeProvider } from '../../theme/provider'
import { supportedLocales, type SupportedLocale } from '../../models/ui'
import { isMockRuntime } from '../../runtime'
import { SelectDialog } from './SelectDialog'
import { parseSelectDialogParams, type SelectDialogInput } from './schemas'

function acceptEmbedder(origin: string): boolean {
  if (isMockRuntime()) return /^https?:\/\/([a-z0-9-]+\.)*(localhost|127\.0\.0\.1)(:\d+)?$/.test(origin)
  const zone = buckyos.getZoneHostName()
  return !!zone && originInZone(origin, zone)
}

function toLocale(locale: string): SupportedLocale {
  const exact = supportedLocales.find(l => l.toLowerCase() === locale.toLowerCase())
  return exact ?? supportedLocales.find(l => l.split('-')[0] === locale.split('-')[0].toLowerCase()) ?? 'en'
}

// One request per document: the App creates the page for a single pick and removes it afterwards.
let view: { init: SelectEmbedInit; input: SelectDialogInput } | null = null
const listeners = new Set<() => void>()
const host = new SelectEmbedHost({
  accept: acceptEmbedder,
  onInit: init => {
    // The embedding App is untrusted input: its parameters go through the same schema as every other caller's.
    try { view = { init, input: parseSelectDialogParams(init.params) } }
    catch { host.error('invalid_params'); return }
    listeners.forEach(fn => fn())
  },
})
const subscribe = (fn: () => void) => { listeners.add(fn); return () => { listeners.delete(fn) } }

function Panel({ input, caller }: { input: SelectDialogInput; caller: string }) {
  const { t } = useI18n()
  const compact = useSyncExternalStore(
    fn => { const query = window.matchMedia('(max-width: 640px)'); query.addEventListener('change', fn); return () => query.removeEventListener('change', fn) },
    () => window.matchMedia('(max-width: 640px)').matches)
  const title = input.title ?? t(`sysdlg.select.title.${input.mode}`)
  return (
    <div className={`flex h-[100dvh] w-full justify-center p-3 sm:p-4 ${compact ? 'items-end' : 'items-center'}`} data-testid="select-embed">
      <section role="dialog" aria-modal="true" aria-label={title}
        className="flex max-h-full w-full max-w-3xl flex-col overflow-hidden rounded-[24px] border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque,var(--cp-surface))] shadow-[0_24px_64px_color-mix(in_srgb,var(--cp-shadow)_22%,transparent)]">
        <header className="relative border-b border-[color:color-mix(in_srgb,var(--cp-border)_74%,transparent)] px-5 pb-4 pt-5 pr-14">
          <button type="button" data-testid="select-close" aria-label={t('common.close', 'Close')} onClick={() => host.result({ action: 'cancelled' })}
            className="absolute right-3 top-3 flex size-8 items-center justify-center rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_82%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] text-[color:var(--cp-muted)] transition-colors duration-150 hover:text-[color:var(--cp-text)]">
            <X className="size-4 stroke-[2]" />
          </button>
          <h2 className="font-display text-xl font-semibold text-[color:var(--cp-text)]">{title}</h2>
          <p className="mt-2 text-sm leading-6 text-[color:var(--cp-muted)]">{t('sysdlg.select.requestedBy', 'Requested by {{name}}', { name: caller })}</p>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5">
          <SelectDialog input={input} callerId={caller} compact={compact} onDone={result => host.result(result)} />
        </div>
      </section>
    </div>
  )
}

/**
 * The system file picker alone, for a standalone App tab to frame over itself (`buckyos.sysdlg-embed`): no Desktop
 * shell. It browses with this origin's own session; the App only receives what the user confirmed.
 */
export function SelectEmbedApp() {
  const current = useSyncExternalStore(subscribe, () => view)
  // The App's page shows through around the dialog.
  useEffect(() => { document.documentElement.style.background = 'transparent'; document.body.style.background = 'transparent' }, [])
  if (!host.parentOrigin) return <p role="alert" className="p-6 text-sm">This page only works inside an App.</p>
  if (!current) return null
  return (
    <PrototypeThemeProvider mode={current.init.theme}>
      <I18nProvider locale={toLocale(current.init.locale)}>
        <Panel input={current.input} caller={new URL(host.parentOrigin).host} />
      </I18nProvider>
    </PrototypeThemeProvider>
  )
}
