import { useCallback, useRef } from 'react'
import { useSudoByPassword, type SudoGrant } from '../../../../components/sudo'
import { useI18n } from '../../../../i18n/provider'
import { isMockRuntime } from '../../../../runtime'

const SUDO_SECONDS = 30 * 60
/** A grant this close to its end is not used for another call. */
const SPARE_MS = 15_000

export interface DomainSudo {
  /** The options of one call, asking for the password when there is no valid grant; null when the user cancels. */
  acquire: () => Promise<{ sessionToken: string | null } | null>
  /** The options without asking; null when there is no valid grant. */
  current: () => { sessionToken: string | null } | null
}

/** One 30-minute grant shared by every step on the page, kept in memory only. */
export function useDomainSudo(): DomainSudo {
  const { t } = useI18n()
  const requestSudo = useSudoByPassword()
  const grant = useRef<SudoGrant | null>(null)

  const current = useCallback(() => {
    if (isMockRuntime()) return { sessionToken: null }
    const held = grant.current
    return held && held.expiresAtMs - SPARE_MS > Date.now() ? { sessionToken: held.sessionToken } : null
  }, [])

  const acquire = useCallback(async () => {
    const held = current()
    if (held) return held
    grant.current = await requestSudo({
      appid: 'control-panel',
      expiresInSecs: SUDO_SECONDS,
      title: t('settings.domain.sudoTitle'),
      description: t('settings.domain.sudoDescription'),
      confirmLabel: t('settings.domain.continue'),
    }).catch(() => null)
    return grant.current ? { sessionToken: grant.current.sessionToken } : null
  }, [current, requestSudo, t])

  return { acquire, current }
}
