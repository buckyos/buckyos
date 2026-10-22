import type { ReactNode } from 'react'
import type { CurrentIdentity } from 'buckyos'
import { Globe, UserRound } from 'lucide-react'
import { useI18n } from '../i18n/provider'
import { identityDisplayName } from './identity'

/** Which kind of identity this is, in visitor words. */
function useKindLabel(identity: CurrentIdentity): string | null {
  const { t } = useI18n()
  if (identity.kind === 'external') return t('sso.identity.external', 'External identity')
  if (identity.kind === 'account' && identity.restricted) return t('sso.identity.limited', 'Restricted account')
  if (identity.kind === 'account') return t('sso.identity.account', 'Account of this Zone')
  return null
}

/** The signed-in identity as the SSO pages show it: name, kind, DID and the Zone it is signed in to. */
export function IdentityCard({ identity, actions, compact = false }: { identity: CurrentIdentity; actions?: ReactNode; compact?: boolean }) {
  const { t } = useI18n()
  const kind = useKindLabel(identity)
  if (identity.kind === 'anonymous') return null
  const name = identityDisplayName(identity)
  return (
    <div
      className="rounded-2xl p-4"
      style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)', border: '1px solid color-mix(in srgb, var(--cp-border) 60%, transparent)' }}
      data-testid="sso-identity-card"
      data-identity-kind={identity.kind}
    >
      <div className="flex items-center gap-3">
        <span
          className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full text-base font-semibold"
          style={{ background: 'color-mix(in srgb, var(--cp-accent) 14%, var(--cp-surface))', color: 'var(--cp-accent)' }}
          aria-hidden
        >
          {name.slice(0, 1).toUpperCase() || <UserRound size={18} />}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }} data-testid="sso-identity-name">
              {name}
            </span>
            {kind ? (
              <span
                className="rounded-full px-2 py-0.5 text-[11px] font-medium"
                style={{ background: 'color-mix(in srgb, var(--cp-accent) 12%, transparent)', color: 'var(--cp-accent)' }}
                data-testid="sso-identity-kind"
              >
                {kind}
              </span>
            ) : null}
          </div>
          {!compact ? (
            <div className="mt-1 space-y-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
              <div className="break-all font-mono" data-testid="sso-identity-did">{identity.did}</div>
              <div className="flex items-center gap-1">
                <Globe size={12} />
                <span>{t('sso.identity.zone', 'Signed in to {{zone}}', { zone: identity.zone_host })}</span>
              </div>
            </div>
          ) : null}
        </div>
        {actions}
      </div>
    </div>
  )
}
