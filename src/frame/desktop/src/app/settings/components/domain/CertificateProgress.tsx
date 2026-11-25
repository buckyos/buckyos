import { Button } from '@mui/material'
import type { HostCertStatus } from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { formatDateTime } from '../shared/format'
import type { CertificateRestart } from './certificates'
import { Failure, Muted } from './parts'

/** How requesting the certificate goes, and the button to request it again. */
export function CertificateProgress({ cert, restart }: { cert: HostCertStatus; restart: CertificateRestart }) {
  const { t, locale } = useI18n()
  const error = cert.acme?.last_error
  const stopped = cert.acme?.state === 'stopped'
  if (!error && !stopped) return null
  return (
    <div className="mt-1 space-y-1" data-testid={`certificate-progress-${cert.host}`}>
      {error ? <Failure failure={{ text: t('settings.domain.cert.lastError', undefined, { when: formatDateTime(error.at, locale) }), detail: error.message }} /> : null}
      {stopped ? <Muted>{t('settings.domain.cert.stopped')}</Muted>
        : cert.acme?.next_retry_at != null ? <Muted>{t('settings.domain.cert.nextRetry', undefined, { when: formatDateTime(cert.acme.next_retry_at, locale) })}</Muted> : null}
      <Button size="small" variant="outlined" disabled={restart.busy !== null} onClick={() => restart.run(cert.host)}>
        {restart.busy === cert.host ? t('settings.domain.cert.restarting') : t('settings.domain.cert.restart')}
      </Button>
      {restart.failed?.host === cert.host ? <Failure failure={restart.failed.failure} /> : null}
    </div>
  )
}
