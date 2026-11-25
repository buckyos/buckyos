/* What a source of the personal site is doing, as the shell says it: in its strip and in the list of websites. */

import { formatMoment } from '../reflow/useReflow'
import type { DisplayState, Feed, SiteSource } from './data'

type Translate = (key: string, fallback?: string, variables?: Record<string, string | number>) => string

/** The tone of the dot beside a source. */
export const SOURCE_TONES: Record<DisplayState, string> = { ok: 'ok', preparing: 'busy', failed: 'failed', needs_login: 'attention', needs_upgrade: 'attention', paused: 'idle' }

/** What a source is doing, in one line; the time is the last check that passed. */
export function sourceText(t: Translate, locale: string, state: DisplayState, source: SiteSource, feed: Feed | null): string {
  const checked = feed?.status.checked_at ?? null
  const variables = { time: checked === null ? '' : formatMoment(checked, locale), note: source.note ?? '' }
  const synced = checked === null ? '' : t('agentBrowser.site.synced', 'Synced {{time}}', variables)
  const withSync = (text: string) => (synced ? `${text} · ${synced}` : text)
  switch (state) {
    case 'ok':
      return synced || t('agentBrowser.site.notSynced', 'Not synced yet')
    case 'preparing':
      return t('agentBrowser.site.preparing', 'Being prepared. Its articles show here after the first sync.')
    case 'paused':
      return withSync(t('agentBrowser.site.paused', 'Paused'))
    case 'needs_login':
      return withSync(t('agentBrowser.site.needsLogin', 'Needs you to sign in on the original site'))
    case 'needs_upgrade':
      return withSync(t('agentBrowser.site.needsUpgrade', 'Not updated until you decide on the upgrade'))
    case 'failed':
      if (source.state === 'failed') return source.note ? t('agentBrowser.site.addFailed', 'Could not be added: {{note}}', variables) : t('agentBrowser.site.addFailedPlain', 'Could not be added')
      return withSync(t('agentBrowser.site.syncFailed', 'The last update failed'))
  }
}
