import { useCallback, useEffect, useState } from 'react'
import { fetchDomainStatus, restartCertificate, type DomainStatus, type HostCertStatus } from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { failureOf, toDate, type DomainFailure, type Tone, type Translate } from './labels'
import type { DomainSudo } from './useDomainSudo'

const SOON_MS = 14 * 86_400_000
/** An attempt takes seconds; what came of it is read once more. */
const OUTCOME_MS = 8_000

export function certificateState(cert: HostCertStatus, t: Translate): { tone: Tone; state: string } {
  if (cert.acme?.state === 'stopped') return { tone: 'fail', state: t('settings.domain.cert.state.stopped') }
  const expires = toDate(cert.not_after)?.getTime()
  if (!cert.loaded) {
    return cert.acme?.state === 'ordering'
      ? { tone: 'warn', state: t('settings.domain.cert.state.ordering') }
      : { tone: 'fail', state: t('settings.domain.cert.state.missing') }
  }
  if (expires !== undefined && expires <= Date.now()) return { tone: 'fail', state: t('settings.domain.cert.state.expired') }
  const late = cert.manager !== 'manual' && expires !== undefined && expires - Date.now() < SOON_MS
  return {
    tone: late || cert.acme?.last_error ? 'warn' : 'pass',
    state: t(cert.acme?.state === 'renewing' ? 'settings.domain.cert.state.renewing' : 'settings.domain.cert.state.valid'),
  }
}

export interface CertificateRestart {
  /** The host being asked for. */
  busy: string | null
  failed: { host: string; failure: DomainFailure } | null
  run: (host: string) => void
}

export function useCertificateRestart(sudo: DomainSudo, accept: (status: DomainStatus) => void): CertificateRestart {
  const { t } = useI18n()
  const [busy, setBusy] = useState<string | null>(null)
  const [failed, setFailed] = useState<CertificateRestart['failed']>(null)
  const [restarted, setRestarted] = useState(0)

  useEffect(() => {
    if (!restarted) return
    let live = true
    const timer = window.setTimeout(() => {
      void fetchDomainStatus().then(({ data }) => { if (live && data) accept(data) })
    }, OUTCOME_MS)
    return () => {
      live = false
      window.clearTimeout(timer)
    }
  }, [restarted, accept])

  const run = useCallback((host: string) => {
    void (async () => {
      const options = await sudo.acquire()
      if (!options) return
      setBusy(host)
      setFailed(null)
      const { data, error } = await restartCertificate(host, options)
      setBusy(null)
      if (data) {
        accept(data)
        setRestarted((count) => count + 1)
      } else setFailed({ host, failure: failureOf(t, error) })
    })()
  }, [accept, sudo, t])

  return { busy, failed, run }
}
