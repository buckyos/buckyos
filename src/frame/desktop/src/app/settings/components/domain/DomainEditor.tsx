import { Button, Radio } from '@mui/material'
import { Save } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import {
  applyDomain,
  planDomain,
  updateDomain,
  type ApplyCredential,
  type CertificateSource,
  type DnsProvider,
  type DomainStatus,
  type SetupIds,
} from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { SectionNote } from '../shared/Section'
import { CertificateUpload } from './CertificateUpload'
import { DomainChecks } from './DomainChecks'
import { ConflictList, RecordCards, RecordList, RecordResults } from './DomainRecords'
import type { DomainDraftState } from './draft'
import { certificateUsable, failureOf, MODES, providerFieldsComplete, providerFieldValues, type DomainFailure } from './labels'
import { Failure, Muted, StepTitle } from './parts'
import { ProviderForm } from './ProviderForm'
import { useDnsProviders } from './useDnsProviders'
import type { DomainSudo } from './useDomainSudo'
import { useVerify } from './useVerify'

function Choice({ checked, disabled, onChoose, title, hint, testId }: {
  checked: boolean
  disabled?: boolean
  onChoose: () => void
  title: string
  hint?: string
  testId: string
}) {
  return (
    <label
      className={`flex items-start gap-2 rounded-lg border p-2 pr-3 ${disabled ? 'cursor-default' : 'cursor-pointer'}`}
      style={{
        borderColor: checked ? 'var(--cp-accent)' : 'var(--cp-border)',
        background: checked ? 'color-mix(in srgb, var(--cp-accent) 8%, transparent)' : undefined,
        opacity: disabled ? 0.6 : 1,
      }}
      data-testid={testId}
    >
      <Radio size="small" checked={checked} disabled={disabled} onChange={onChoose} slotProps={{ input: { 'aria-label': title } }} />
      <span className="min-w-0 pt-1.5">
        <span className="block text-sm font-medium leading-6" style={{ color: 'var(--cp-text)' }}>{title}</span>
        {hint ? <span className="block text-sm leading-5" style={{ color: 'var(--cp-muted)' }}>{hint}</span> : null}
      </span>
    </label>
  )
}

function Step({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="space-y-3">
      <StepTitle>{title}</StepTitle>
      {children}
    </div>
  )
}

/** Step one of changing the hosting: who manages the DNS records and where the certificate comes from. */
function ChooseStep({ state, sudo, onCancel }: { state: DomainDraftState; sudo: DomainSudo; onCancel: () => void }) {
  const { t } = useI18n()
  const draft = state.draft!
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<DomainFailure | null>(null)
  const sources: CertificateSource[] = ['acme', 'upload']

  const proceed = async () => {
    const options = await sudo.acquire()
    if (!options) return
    setBusy(true)
    setFailure(null)
    const { data, error } = await planDomain({ mode: draft.mode, certificate: draft.certificate }, options)
    setBusy(false)
    if (data) state.patch({ step: 'setup', plan: data, applied: null })
    else setFailure(failureOf(t, error))
  }

  return (
    <div className="space-y-5" data-testid="domain-choose">
      <Step title={t('settings.domain.dns')}>
        <div role="radiogroup" aria-label={t('settings.domain.dns')} className="space-y-2">
          {MODES.map((mode) => (
            <Choice
              key={mode}
              checked={draft.mode === mode}
              disabled={busy}
              onChoose={() => state.patch({ mode, ...(mode === 'manual' ? { certificate: 'upload' as const } : {}) })}
              title={t(`settings.domain.mode.${mode}`)}
              hint={t(`settings.domain.modeHint.${mode}`)}
              testId={`domain-mode-${mode}`}
            />
          ))}
        </div>
      </Step>
      <Step title={t('settings.domain.certificate')}>
        <div role="radiogroup" aria-label={t('settings.domain.certificate')} className="space-y-2">
          {sources.map((source) => {
            const manualOnly = source === 'acme' && draft.mode === 'manual'
            return (
              <Choice
                key={source}
                checked={draft.certificate === source}
                disabled={busy || manualOnly}
                onChoose={() => state.patch({ certificate: source })}
                title={t(`settings.domain.source.${source}`)}
                hint={t(manualOnly ? 'settings.domain.sourceManualOnly' : `settings.domain.sourceHint.${source}`)}
                testId={`domain-source-${source}`}
              />
            )
          })}
        </div>
      </Step>
      <Failure failure={failure} />
      <div className="flex flex-wrap gap-2">
        <Button size="small" variant="contained" disabled={busy} onClick={() => void proceed()} data-testid="domain-continue">{t('settings.domain.continue')}</Button>
        <Button size="small" variant="text" disabled={busy} onClick={onCancel}>{t('settings.common.cancel')}</Button>
      </div>
    </div>
  )
}

/** The token, the records the node will change, and what came of writing them. */
function ApiRecords({ status, state, sudo, providers }: { status: DomainStatus; state: DomainDraftState; sudo: DomainSudo; providers: DnsProvider[] }) {
  const { t } = useI18n()
  const draft = state.draft!
  const [providerId, setProviderId] = useState(status.credential?.provider ?? '')
  const [fields, setFields] = useState<Record<string, string>>({})
  const [confirmed, setConfirmed] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<DomainFailure | null>(null)
  const provider = providers.find((item) => item.id === providerId) ?? null
  const saved = draft.intent === 'hosting' ? status.credential : null
  const savedName = providers.find((item) => item.id === saved?.provider)?.name ?? saved?.provider ?? ''
  const keep = Boolean(saved && draft.keepToken)
  const written = draft.applied?.credential_id ?? null
  const conflicts = draft.applied ? draft.applied.conflicts : draft.plan?.conflicts ?? []

  const write = async () => {
    const credential: ApplyCredential | null = keep && saved
      ? { credential_id: saved.id }
      : provider && providerFieldsComplete(provider, fields)
        ? { provider: provider.id, fields: providerFieldValues(provider, fields) }
        : written ? { credential_id: written } : null
    if (!credential) return
    const options = await sudo.acquire()
    if (!options) return
    setBusy(true)
    setFailure(null)
    const { data, error } = await applyDomain({ mode: draft.mode, certificate: draft.certificate }, credential, confirmed, options)
    setBusy(false)
    if (!data) {
      setFailure(failureOf(t, error))
      return
    }
    if (data.credential_id) setFields({})
    state.patch({ applied: data })
  }

  const ready = keep || providerFieldsComplete(provider, fields) || Boolean(written)
  const change = (patch: Parameters<DomainDraftState['patch']>[0]) => {
    setConfirmed([])
    setFields({})
    state.patch({ ...patch, applied: null })
  }

  return (
    <>
      <Step title={t('settings.domain.token')}>
        {saved ? (
          <div role="radiogroup" aria-label={t('settings.domain.token')} className="space-y-2">
            <Choice checked={keep} disabled={busy} onChoose={() => change({ keepToken: true })} title={t('settings.domain.tokenKeep', undefined, { provider: savedName })} testId="domain-token-keep" />
            <Choice checked={!keep} disabled={busy} onChoose={() => change({ keepToken: false })} title={t('settings.domain.tokenNew')} testId="domain-token-new" />
          </div>
        ) : null}
        {keep ? null : written ? (
          <div className="flex flex-wrap items-center gap-2">
            <Muted>{t('settings.domain.tokenAccepted')}</Muted>
            <Button size="small" variant="text" disabled={busy} onClick={() => change({})}>{t('settings.domain.tokenNew')}</Button>
          </div>
        ) : (
          <ProviderForm
            providers={providers}
            provider={provider}
            onProvider={(id) => { setProviderId(id); setFields({}) }}
            fields={fields}
            onFields={setFields}
            disabled={busy}
          />
        )}
      </Step>
      <Step title={t(draft.applied ? 'settings.domain.dns' : 'settings.domain.recordsApi')}>
        {draft.applied ? <RecordResults results={draft.applied.results} /> : draft.plan ? <RecordList records={draft.plan.records} /> : null}
        <ConflictList conflicts={conflicts} confirmed={confirmed} onConfirm={setConfirmed} />
        <Failure failure={failure} />
        <Button size="small" variant={written && !conflicts.length ? 'outlined' : 'contained'} disabled={busy || !ready} onClick={() => void write()} data-testid="domain-write">
          {busy ? t('settings.domain.writing') : t('settings.domain.write')}
        </Button>
      </Step>
    </>
  )
}

/** Step two: the records, the certificate and the checks; saving is open once everything needed is there. */
function SetupStep({ status, state, sudo, onSaved, onCancel }: {
  status: DomainStatus
  state: DomainDraftState
  sudo: DomainSudo
  onSaved: (status: DomainStatus) => void
  onCancel: () => void
}) {
  const { t } = useI18n()
  const draft = state.draft!
  const { intent, mode, certificate, plan, applied, uploaded } = draft
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<DomainFailure | null>(null)
  const needsProviders = mode === 'dns_api' && intent !== 'certificate'
  const { providers, error: providersError } = useDnsProviders(needsProviders)
  const needsCertificate = certificate === 'upload' && intent !== 'token'
  const keepCertificate = needsCertificate && draft.keepCertificate
  const credentialId = needsProviders ? applied?.credential_id ?? undefined : undefined
  const certificateId = needsCertificate && !keepCertificate ? uploaded?.id : undefined
  const ids: SetupIds = {
    ...(credentialId ? { credential_id: credentialId } : {}),
    ...(certificateId ? { certificate_id: certificateId } : {}),
  }
  // Replacing the token only needs the provider to accept it.
  const recordsWritten = !needsProviders
    || Boolean(applied?.credential_id && (intent === 'token' || applied.results.every((result) => result.status !== 'failed')))
  const verifying = intent === 'hosting' && recordsWritten
  const verify = useVerify(verifying && !busy, { mode, certificate }, ids, sudo)
  const ownershipPassed = verify.check?.items.some((item) => item.id === 'ownership' && item.status === 'pass') ?? false
  const certificateReady = !needsCertificate || keepCertificate || certificateUsable(uploaded)
  const ready = certificateReady && recordsWritten && (intent !== 'hosting' || Boolean(verify.check?.ready))

  const save = async () => {
    const options = await sudo.acquire()
    if (!options) return
    setBusy(true)
    setFailure(null)
    const { data, error } = await updateDomain({ mode, certificate }, ids, Boolean(certificateId && uploaded?.accepted), options)
    setBusy(false)
    if (data) onSaved(data)
    else setFailure(failureOf(t, error))
  }

  // The SN takes the delegation only after it has confirmed who owns the domain.
  const records = plan?.records.filter((record) => mode !== 'sn' || record.phase === 'ownership' || ownershipPassed) ?? []
  const nsWaits = mode === 'sn' && !ownershipPassed && (plan?.records.some((record) => record.phase === 'routing') ?? false)

  return (
    <div className="space-y-5" data-testid="domain-setup">
      {needsProviders
        ? providers ? <ApiRecords status={status} state={state} sudo={sudo} providers={providers} />
          : providersError ? <Failure failure={failureOf(t, providersError)} /> : <Muted>{t('settings.common.loading')}</Muted>
        : intent === 'hosting' && plan ? (
          <Step title={t('settings.domain.records')}>
            <Muted>{t('settings.domain.recordsHint')}</Muted>
            <RecordCards records={records} />
            {nsWaits ? <Muted>{t('settings.domain.recordsNsLater')}</Muted> : null}
            <ConflictList conflicts={plan.conflicts} />
          </Step>
        ) : null}

      {needsCertificate ? (
        <Step title={t('settings.domain.certificate')}>
          {intent === 'hosting' && status.hosting?.certificate === 'upload' ? (
            <div role="radiogroup" aria-label={t('settings.domain.certificate')} className="space-y-2">
              <Choice checked={keepCertificate} disabled={busy} onChoose={() => state.patch({ keepCertificate: true })} title={t('settings.domain.cert.keep')} testId="domain-certificate-keep" />
              <Choice checked={!keepCertificate} disabled={busy} onChoose={() => state.patch({ keepCertificate: false })} title={t('settings.domain.cert.upload')} testId="domain-certificate-new" />
            </div>
          ) : null}
          {keepCertificate ? null : (
            <CertificateUpload domain={status.hostname} sudo={sudo} value={uploaded} onChange={(value) => state.patch({ uploaded: value })} />
          )}
          <Muted>{t('settings.domain.sourceHint.upload')}</Muted>
        </Step>
      ) : null}

      {verifying ? (
        <Step title={t('settings.domain.checks')}>
          {verify.check ? <DomainChecks items={verify.check.items} /> : verify.error || verify.expired ? null : <Muted>{t('settings.domain.checking')}</Muted>}
          {verify.error ? <Failure failure={failureOf(t, verify.error)} /> : null}
          {verify.expired ? <SectionNote tone="warning">{t('settings.domain.sudoExpired')}</SectionNote> : null}
          {verify.check?.ready ? null : <Muted>{t('settings.domain.checksHint')}</Muted>}
          <Button size="small" variant="outlined" disabled={verify.busy} onClick={() => void verify.checkNow()} data-testid="domain-verify">
            {verify.busy ? t('settings.domain.checking') : t('settings.domain.checkAgain')}
          </Button>
        </Step>
      ) : null}

      <Failure failure={failure} testId="domain-save-failure" />
      {busy && intent === 'hosting' ? <Muted>{t('settings.domain.savingHint')}</Muted> : null}
      <div className="flex flex-wrap gap-2">
        <Button size="small" variant="contained" startIcon={<Save size={14} />} disabled={busy || !ready} onClick={() => void save()} data-testid="domain-save">
          {busy ? t('settings.zone.saving') : t('settings.zone.save')}
        </Button>
        {intent === 'hosting' ? (
          <Button size="small" variant="text" disabled={busy} onClick={() => state.patch({ step: 'choose', applied: null })} data-testid="domain-back">{t('settings.domain.back')}</Button>
        ) : null}
        <Button size="small" variant="text" disabled={busy} onClick={onCancel} data-testid="domain-cancel">{t('settings.common.cancel')}</Button>
      </div>
    </div>
  )
}

/** Changing the hosting, or replacing the token or the certificate, in place of the section's status. */
export function DomainEditor({ status, state, sudo, onSaved }: {
  status: DomainStatus
  state: DomainDraftState
  sudo: DomainSudo
  onSaved: (status: DomainStatus) => void
}) {
  const draft = state.draft
  if (!draft) return null
  return draft.step === 'choose'
    ? <ChooseStep state={state} sudo={sudo} onCancel={state.clear} />
    : <SetupStep status={status} state={state} sudo={sudo} onSaved={onSaved} onCancel={state.clear} />
}
