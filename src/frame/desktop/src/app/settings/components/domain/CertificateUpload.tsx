import { Button, Checkbox, FormControlLabel, IconButton, TextField } from '@mui/material'
import { ClipboardPaste, FileUp, X } from 'lucide-react'
import { useState } from 'react'
import { inspectDomainCertificate, type CertificateCheckLevel, type CertificateInspection } from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { usePickFiles } from '../../../../sysdlg/select/pickFiles'
import { formatBytes, formatDateTime } from '../shared/format'
import { SectionNote } from '../shared/Section'
import {
  certificateCheckLabel,
  certificateReasonText,
  failureOf,
  hasCertificateWarning,
  isPrivateCertificate,
  type CheckedCertificate,
  type DomainFailure,
  type Tone,
} from './labels'
import { Failure, Mono, Muted, StatusLine } from './parts'
import type { DomainSudo } from './useDomainSudo'

const MAX_TOTAL_BYTES = 1024 * 1024
const LEVEL_TONES: Record<CertificateCheckLevel, Tone> = { pass: 'pass', warn: 'warn', reject: 'fail' }

interface ChosenFile {
  name: string
  bytes: Uint8Array
}

function base64(bytes: Uint8Array): string {
  let binary = ''
  for (let start = 0; start < bytes.length; start += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(start, start + 0x8000))
  }
  return btoa(binary)
}

/** The node's checks, one by one, and which of the Zone's two names the certificate covers. */
export function InspectionResult({ inspection }: { inspection: CertificateInspection }) {
  const { t, locale } = useI18n()
  const { domain } = inspection
  const invalidTime = inspection.items.some((item) => item.code === 'validity' && item.level === 'reject')
  return (
    <div className="space-y-2" data-testid="domain-certificate-result">
      <div className="divide-y divide-[color:var(--cp-border)]">
        {inspection.items.map((item) => (
          <StatusLine
            key={item.code}
            tone={LEVEL_TONES[item.level]}
            title={certificateCheckLabel(t, item.code, domain)}
            state={t(`settings.domain.level.${item.level}`)}
            testId={`domain-certificate-${item.code}`}
          >
            {item.level === 'pass' ? null : (
              <>
                <Muted>{certificateReasonText(t, item.reason, domain)}</Muted>
                {item.detail ? <Mono>{item.detail}</Mono> : null}
              </>
            )}
          </StatusLine>
        ))}
      </div>
      {inspection.coverage.length ? (
        <dl className="space-y-1 text-xs leading-5">
          {inspection.coverage.map((name) => (
            <div key={name.name} className="flex flex-wrap gap-x-2">
              <dt><Mono>{name.name}</Mono></dt>
              <dd style={{ color: name.covered_by ? 'var(--cp-muted)' : 'var(--cp-danger)' }}>
                {name.covered_by ? t('settings.domain.cert.coveredBy', undefined, { name: name.covered_by }) : t('settings.domain.cert.notCovered')}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
      {inspection.not_after ? (
        <Muted>
          {t('settings.domain.cert.expires', undefined, { date: formatDateTime(inspection.not_after, locale) })}
          {inspection.issuer ? ` · ${inspection.issuer}` : ''}
        </Muted>
      ) : null}
      {invalidTime ? <Muted>{t('settings.domain.cert.nodeTime', undefined, { time: formatDateTime(inspection.node_time, locale) })}</Muted> : null}
    </div>
  )
}

/**
 * Choosing a certificate (files from the Zone or this device, or pasted text), the node's checks and the user's
 * agreement to their warnings. What was chosen stays in memory until the node has checked it.
 */
export function CertificateUpload({ domain, sudo, value, onChange }: {
  domain: string
  sudo: DomainSudo
  value: CheckedCertificate | null
  onChange: (value: CheckedCertificate | null) => void
}) {
  const { t } = useI18n()
  const pickFiles = usePickFiles('settings.certificate')
  const [files, setFiles] = useState<ChosenFile[]>([])
  const [pasting, setPasting] = useState(false)
  const [pasted, setPasted] = useState('')
  const [password, setPassword] = useState('')
  const [busy, setBusy] = useState(false)
  const [rejected, setRejected] = useState<CertificateInspection | null>(null)
  const [failure, setFailure] = useState<DomainFailure | null>(null)

  const passwordWrong = rejected?.items.some((item) => item.reason === 'pfx_password_wrong') ?? false
  const needsPassword = passwordWrong || files.some((file) => /\.(pfx|p12)$/i.test(file.name))
  const total = files.reduce((sum, file) => sum + file.bytes.length, pasted.length)
  const tooLarge = total > MAX_TOTAL_BYTES

  const changed = () => {
    setRejected(null)
    setFailure(null)
  }

  const choose = async () => {
    try {
      const picked = await pickFiles({ multiple: true, title: t('settings.domain.cert.choose') })
      if (!picked?.length) return
      if (picked.reduce((sum, file) => sum + file.size, total) > MAX_TOTAL_BYTES) {
        setFailure({ text: t('settings.domain.cert.tooLarge'), detail: null })
        return
      }
      const read = await Promise.all(picked.map(async (file) => ({ name: file.name, bytes: new Uint8Array(await file.arrayBuffer()) })))
      changed()
      setFiles((current) => [...current.filter((file) => !read.some((next) => next.name === file.name)), ...read])
    } catch (error) {
      setFailure({ text: t('settings.domain.cert.readFailed'), detail: error instanceof Error ? error.message : String(error) })
    }
  }

  const check = async () => {
    const options = await sudo.acquire()
    if (!options) return
    setBusy(true)
    changed()
    const contents = files.map((file) => base64(file.bytes))
    if (pasted.trim()) contents.push(base64(new TextEncoder().encode(pasted)))
    const { data, error } = await inspectDomainCertificate(
      { files: contents, ...(needsPassword && password ? { pfx_password: password } : {}) },
      options,
    )
    setBusy(false)
    if (!data) {
      setFailure(failureOf(t, error))
    } else if (!data.certificate_id) {
      setRejected(data.inspection)
    } else {
      setFiles([])
      setPasted('')
      setPassword('')
      onChange({ id: data.certificate_id, inspection: data.inspection, accepted: false })
    }
  }

  if (value) {
    const warned = hasCertificateWarning(value.inspection)
    return (
      <div className="space-y-3" data-testid="domain-certificate">
        <SectionNote tone={warned ? 'warning' : 'muted'}>{t(warned ? 'settings.domain.cert.warned' : 'settings.domain.cert.passed')}</SectionNote>
        <InspectionResult inspection={value.inspection} />
        {warned ? (
          <FormControlLabel
            sx={{ alignItems: 'flex-start', mr: 0 }}
            control={(
              <Checkbox
                size="small"
                checked={value.accepted}
                onChange={(event) => onChange({ ...value, accepted: event.target.checked })}
                data-testid="domain-certificate-accept"
              />
            )}
            label={(
              <span className="block pt-2 text-sm leading-6" style={{ color: 'var(--cp-text)' }}>
                {t(isPrivateCertificate(value.inspection) ? 'settings.domain.cert.acceptPrivate' : 'settings.domain.cert.accept')}
              </span>
            )}
          />
        ) : null}
        <Button size="small" variant="outlined" onClick={() => { onChange(null); changed() }}>{t('settings.domain.cert.another')}</Button>
      </div>
    )
  }

  return (
    <div className="space-y-3" data-testid="domain-certificate">
      <Muted>{t('settings.domain.cert.names', undefined, { domain })}</Muted>
      <Muted>{t('settings.domain.cert.formats')}</Muted>
      <div className="flex flex-wrap gap-2">
        <Button size="small" variant="outlined" startIcon={<FileUp size={14} />} disabled={busy} onClick={() => void choose()} data-testid="domain-certificate-choose">
          {t('settings.domain.cert.choose')}
        </Button>
        <Button size="small" variant="outlined" startIcon={<ClipboardPaste size={14} />} disabled={busy || pasting} onClick={() => setPasting(true)} data-testid="domain-certificate-paste">
          {t('settings.domain.cert.paste')}
        </Button>
      </div>
      {files.length ? (
        <ul className="space-y-1">
          {files.map((file) => (
            <li key={file.name} className="flex items-center gap-2 text-sm" style={{ color: 'var(--cp-text)' }}>
              <span className="min-w-0 break-all">{file.name}</span>
              <span className="shrink-0 text-xs" style={{ color: 'var(--cp-muted)' }}>{formatBytes(file.bytes.length)}</span>
              <IconButton
                size="small"
                disabled={busy}
                aria-label={t('settings.domain.cert.remove', undefined, { name: file.name })}
                onClick={() => { changed(); setFiles((current) => current.filter((item) => item !== file)) }}
              >
                <X size={14} />
              </IconButton>
            </li>
          ))}
        </ul>
      ) : null}
      {pasting ? (
        <TextField
          fullWidth
          multiline
          minRows={4}
          maxRows={10}
          size="small"
          label={t('settings.domain.cert.pasteLabel')}
          placeholder="-----BEGIN CERTIFICATE-----"
          value={pasted}
          disabled={busy}
          autoComplete="off"
          onChange={(event) => { changed(); setPasted(event.target.value) }}
          slotProps={{
            inputLabel: { shrink: true },
            htmlInput: { dir: 'ltr', spellCheck: false, autoCapitalize: 'off', autoCorrect: 'off', className: 'font-mono text-xs', 'data-testid': 'domain-certificate-text' },
          }}
        />
      ) : null}
      {needsPassword ? (
        <TextField
          fullWidth
          size="small"
          type="password"
          autoComplete="off"
          label={t('settings.domain.cert.password')}
          value={password}
          disabled={busy}
          onChange={(event) => setPassword(event.target.value)}
          slotProps={{ htmlInput: { 'data-testid': 'domain-certificate-password' } }}
        />
      ) : null}
      {tooLarge ? <SectionNote tone="danger">{t('settings.domain.cert.tooLarge')}</SectionNote> : null}
      <Failure failure={failure} />
      {rejected ? (
        <>
          <SectionNote tone="danger">{t('settings.domain.cert.rejected')}</SectionNote>
          <InspectionResult inspection={rejected} />
        </>
      ) : null}
      <Button
        size="small"
        variant="contained"
        disabled={busy || tooLarge || (!files.length && !pasted.trim())}
        onClick={() => void check()}
        data-testid="domain-certificate-check"
      >
        {busy ? t('settings.domain.checking') : t('settings.domain.cert.check')}
      </Button>
    </div>
  )
}
