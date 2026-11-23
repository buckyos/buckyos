import { Button, IconButton, Tooltip } from '@mui/material'
import { RefreshCw } from 'lucide-react'
import { useCallback, useEffect, useRef, useState } from 'react'
import { checkDomain, domainErrorOf, fetchDomainStatus, planDomain, type DomainStatus, type HostCertStatus } from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { formatDateTime, formatRelative } from '../shared/format'
import { InfoRow, Section, SectionNote } from '../shared/Section'
import { DomainChecks } from './DomainChecks'
import { DomainEditor } from './DomainEditor'
import { newDraft, useDomainDraft, type EditorIntent } from './draft'
import { certificateSourceText, failureOf, problemText, toDate, type DomainFailure, type Tone, type Translate } from './labels'
import { Failure, Mono, Muted, StatusLine, StepTitle } from './parts'
import { useDnsProviders } from './useDnsProviders'
import { useDomainSudo } from './useDomainSudo'

const SOON_MS = 14 * 86_400_000

function certificateState(cert: HostCertStatus, t: Translate): { tone: Tone; state: string } {
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

function CertificateRow({ cert }: { cert: HostCertStatus }) {
  const { t, locale } = useI18n()
  const { tone, state } = certificateState(cert, t)
  const error = cert.acme?.last_error
  const facts = [
    cert.not_after != null ? t('settings.domain.cert.expires', undefined, { date: formatDateTime(cert.not_after, locale) }) : null,
    cert.issuer,
    cert.manager ? t(`settings.domain.manager.${cert.manager}`) : null,
  ].filter(Boolean)
  return (
    <StatusLine tone={tone} title={<Mono>{cert.host}</Mono>} state={state} testId={`domain-cert-${cert.host}`}>
      {facts.length ? <Muted>{facts.join(' · ')}</Muted> : null}
      {error ? (
        <div className="mt-1">
          <Failure failure={{ text: t('settings.domain.cert.lastError', undefined, { when: formatDateTime(error.at, locale) }), detail: error.message }} />
          {cert.acme?.next_retry_at != null ? <Muted>{t('settings.domain.cert.nextRetry', undefined, { when: formatDateTime(cert.acme.next_retry_at, locale) })}</Muted> : null}
        </div>
      ) : null}
    </StatusLine>
  )
}

function Certificates({ status }: { status: DomainStatus }) {
  const { t } = useI18n()
  return (
    <div>
      <StepTitle>{t('settings.domain.certificates')}</StepTitle>
      {status.certificates.length ? (
        <div className="divide-y divide-[color:var(--cp-border)]">
          {status.certificates.map((cert) => <CertificateRow key={cert.host} cert={cert} />)}
        </div>
      ) : <Muted>{t('settings.domain.certificatesNone')}</Muted>}
    </div>
  )
}

/** Settings › Zone Config › Domain and certificate, for the Zone owner. */
export function ZoneDomainSection() {
  const { t, locale } = useI18n()
  const sudo = useDomainSudo()
  const draftState = useDomainDraft()
  const [status, setStatus] = useState<DomainStatus | null>(null)
  const [access, setAccess] = useState<'loading' | 'ready' | 'denied' | 'failed'>('loading')
  const [loadFailure, setLoadFailure] = useState<DomainFailure | null>(null)
  const [busy, setBusy] = useState<'refresh' | 'check' | 'open' | null>(null)
  const [failure, setFailure] = useState<DomainFailure | null>(null)
  const [savedNote, setSavedNote] = useState(false)
  const section = useRef<HTMLDivElement>(null)

  const accept = useCallback(({ data, error }: Awaited<ReturnType<typeof fetchDomainStatus>>) => {
    if (data) {
      setStatus(data)
      setAccess('ready')
      setLoadFailure(null)
      return
    }
    const { code } = domainErrorOf(error)
    if (code === 'not_owner' || code === 'session_not_allowed') setAccess('denied')
    else {
      setAccess('failed')
      setLoadFailure(failureOf(t, error))
    }
  }, [t])

  useEffect(() => {
    let live = true
    void fetchDomainStatus().then((result) => { if (live) accept(result) })
    return () => { live = false }
  }, [accept])

  const refresh = async () => {
    setBusy('refresh')
    accept(await fetchDomainStatus())
    setBusy(null)
  }

  const check = async () => {
    const options = await sudo.acquire()
    if (!options) return
    setBusy('check')
    setFailure(null)
    const { data, error } = await checkDomain(options)
    setBusy(null)
    if (data) setStatus((current) => current && { ...current, check: data })
    else setFailure(failureOf(t, error))
  }

  const open = async (intent: EditorIntent) => {
    if (!status?.hosting) return
    setSavedNote(false)
    setFailure(null)
    const draft = newDraft(intent, status)
    if (intent === 'token') {
      const options = await sudo.acquire()
      if (!options) return
      setBusy('open')
      const { data, error } = await planDomain({ mode: draft.mode, certificate: draft.certificate }, options)
      setBusy(null)
      if (!data) {
        setFailure(failureOf(t, error))
        return
      }
      draft.plan = data
    }
    draftState.start(draft)
    section.current?.scrollIntoView({ block: 'start' })
  }

  const hosting = status?.hosting ?? null
  const editing = hosting && draftState.draft
  const { providers } = useDnsProviders(Boolean(hosting?.provider))
  const providerName = providers?.find((item) => item.id === hosting?.provider)?.name ?? hosting?.provider
  const title = editing ? t(`settings.domain.title.${editing.intent}`) : t('settings.domain.title')

  return (
    <div ref={section}>
      <Section
        title={title}
        testId="settings-zone-domain"
        actions={access === 'ready' && !editing ? (
          <Tooltip title={t('settings.domain.refresh')}>
            <IconButton size="small" aria-label={t('settings.domain.refresh')} disabled={busy !== null} onClick={() => void refresh()}>
              <RefreshCw size={16} />
            </IconButton>
          </Tooltip>
        ) : undefined}
      >
        {access === 'loading' ? <SectionNote>{t('settings.common.loading')}</SectionNote>
          : access === 'denied' ? <SectionNote>{t('settings.domain.ownerOnly')}</SectionNote>
            : access === 'failed' || !status ? (
              <Failure
                failure={loadFailure ?? { text: t('settings.domain.loadFailed'), detail: null }}
                onRetry={() => { setAccess('loading'); void refresh() }}
              />
            ) : editing ? (
              <DomainEditor
                key={editing.intent}
                status={status}
                state={draftState}
                sudo={sudo}
                onSaved={(next) => { setStatus(next); draftState.clear(); setSavedNote(true) }}
              />
            ) : (
              <div className="space-y-5">
                {savedNote ? (
                  <p role="status" className="text-sm leading-6" style={{ color: 'var(--cp-success)' }} data-testid="domain-saved">
                    {t(hosting?.certificate === 'acme' ? 'settings.domain.savedAcme' : 'settings.domain.saved')}
                  </p>
                ) : null}
                {status.problems.length ? (
                  <div data-testid="domain-problems">
                    {status.problems.map((problem) => <SectionNote key={problem} tone="danger">{problemText(t, problem)}</SectionNote>)}
                  </div>
                ) : null}

                <div className="space-y-0.5">
                  <InfoRow label={t('settings.domain.hostname')} value={<Mono>{status.hostname}</Mono>} />
                  {hosting ? (
                    <>
                      <InfoRow label={t('settings.domain.dns')} value={t(`settings.domain.mode.${hosting.mode}`)} />
                      {providerName ? <InfoRow label={t('settings.domain.provider')} value={providerName} /> : null}
                      <InfoRow label={t('settings.domain.certificate')} value={certificateSourceText(t, hosting)} />
                    </>
                  ) : null}
                </div>
                {hosting ? null : <Muted>{t('settings.domain.defaultHint')}</Muted>}

                <Certificates status={status} />

                {hosting ? (
                  <>
                    <div>
                      <div className="flex flex-wrap items-center justify-between gap-2">
                        <StepTitle>{t('settings.domain.checks')}</StepTitle>
                        <Button size="small" variant="outlined" disabled={busy !== null} onClick={() => void check()} data-testid="domain-check">
                          {busy === 'check' ? t('settings.domain.checking') : t('settings.domain.checkAgain')}
                        </Button>
                      </div>
                      {status.check ? (
                        <>
                          <Muted>{t('settings.domain.checkedAt', undefined, { when: formatRelative(status.check.checked_at, locale) })}</Muted>
                          <DomainChecks items={status.check.items} />
                        </>
                      ) : <Muted>{t('settings.domain.checksNever')}</Muted>}
                    </div>
                    <Failure failure={failure} />
                    <div className="flex flex-wrap gap-2">
                      {hosting.mode === 'dns_api' ? (
                        <Button size="small" variant="outlined" disabled={busy !== null} onClick={() => void open('token')} data-testid="domain-replace-token">{t('settings.domain.title.token')}</Button>
                      ) : null}
                      {hosting.certificate === 'upload' ? (
                        <Button size="small" variant="outlined" disabled={busy !== null} onClick={() => void open('certificate')} data-testid="domain-replace-certificate">{t('settings.domain.title.certificate')}</Button>
                      ) : null}
                      <Button size="small" variant="outlined" disabled={busy !== null} onClick={() => void open('hosting')} data-testid="domain-change">{t('settings.domain.title.hosting')}</Button>
                    </div>
                  </>
                ) : null}
              </div>
            )}
      </Section>
    </div>
  )
}
