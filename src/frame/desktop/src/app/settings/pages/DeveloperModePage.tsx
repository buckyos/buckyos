import { AlertTriangle, CheckCircle, Download, FileText, Loader2, RefreshCw, Terminal, XCircle } from 'lucide-react'
import { Button, IconButton, Switch, TextField, Tooltip } from '@mui/material'
import { useEffect, useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useSudoByPassword } from '../../../components/sudo'
import { isMockRuntime } from '../../../runtime'
import {
  fetchGatewayFile,
  fetchGatewayOverview,
  fetchLogServices,
  requestLogDownload,
  type ConfigFileSummary,
  type GatewayFileResponse,
  type ZoneCertificatesResponse,
  type ZoneDevicesResponse,
  type ZoneOverviewResponse,
} from '../../../api/settings'
import { useSettingsResource, useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'
import type { Resource } from '../data/store'
import { InfoRow, ListCard, Section, SectionNote } from '../components/shared/Section'
import { CopyButton } from '../components/shared/CopyButton'
import { downloadUrl, formatBytes, formatDateTime } from '../components/shared/format'

type Translate = ReturnType<typeof useI18n>['t']
type DiagnosticStatus = 'pass' | 'warn' | 'fail' | 'pending'

interface Diagnostic {
  key: string
  name: string
  status: DiagnosticStatus
  message: string
}

const diagnosticIcons = {
  pass: CheckCircle,
  warn: AlertTriangle,
  fail: XCircle,
  pending: Loader2,
} as const

const diagnosticColors: Record<DiagnosticStatus, string> = {
  pass: 'var(--cp-success)',
  warn: 'var(--cp-warning)',
  fail: 'var(--cp-danger)',
  pending: 'var(--cp-muted)',
}

/** Checks derived from what the Zone reports; nothing here is probed separately. */
function buildDiagnostics(
  devices: Resource<ZoneDevicesResponse>,
  certificates: Resource<ZoneCertificatesResponse>,
  overview: Resource<ZoneOverviewResponse>,
  t: Translate,
): Diagnostic[] {
  const result: Diagnostic[] = []
  const pending = (key: string, name: string) => ({ key, name, status: 'pending' as const, message: t('settings.common.loading', 'Loading…') })
  const failed = (key: string, name: string, error: string | null) => ({ key, name, status: 'fail' as const, message: error ?? '' })

  const nodesName = t('settings.developer.diag.nodes', 'Nodes')
  if (devices.data) {
    const oods = devices.data.devices.filter((device) => device.is_ood)
    const offline = oods.filter((device) => !device.online).map((device) => device.name)
    result.push({
      key: 'nodes',
      name: nodesName,
      status: offline.length === 0 ? 'pass' : offline.length < oods.length ? 'warn' : 'fail',
      message: offline.length === 0
        ? t('settings.developer.diag.nodesOk', 'All {{count}} OOD nodes are reporting.', { count: oods.length })
        : t('settings.developer.diag.nodesOffline', 'Not reporting: {{nodes}}', { nodes: offline.join(', ') }),
    })
    const local = devices.data.devices.find((device) => device.is_local)
    const total = local?.storage?.total_bytes
    const used = local?.storage?.used_bytes
    if (local && total && used != null) {
      const percent = Math.round((used / total) * 100)
      result.push({
        key: 'storage',
        name: t('settings.developer.diag.storage', 'Storage'),
        status: percent >= 90 ? 'fail' : percent >= 80 ? 'warn' : 'pass',
        message: t('settings.developer.diag.storageUsage', '{{node}}: {{used}} of {{total}} used ({{percent}}%)', {
          node: local.name,
          used: formatBytes(used),
          total: formatBytes(total),
          percent,
        }),
      })
    }
  } else {
    result.push(devices.status === 'error' ? failed('nodes', nodesName, devices.error) : pending('nodes', nodesName))
  }

  const certName = t('settings.developer.diag.certificate', 'TLS Certificate')
  if (certificates.data) {
    const certs = certificates.data.certificates
    const worst = certs.reduce<number | null>((min, cert) => (cert.days_remaining == null ? min : min == null ? cert.days_remaining : Math.min(min, cert.days_remaining)), null)
    const broken = certs.filter((cert) => cert.state !== 'valid')
    result.push({
      key: 'certificate',
      name: certName,
      status: certs.length === 0 ? 'warn' : broken.length > 0 ? 'fail' : worst != null && worst < 14 ? 'warn' : 'pass',
      message: certs.length === 0
        ? t('settings.developer.diag.noCertificate', 'No certificate installed on this node.')
        : broken.length > 0
          ? t('settings.developer.diag.certificateBroken', 'Not usable: {{hosts}}', { hosts: broken.map((cert) => cert.host).join(', ') })
          : t('settings.developer.diag.certificateOk', 'Valid; the earliest expires in {{days}} days.', { days: worst ?? '—' }),
    })
  } else {
    result.push(certificates.status === 'error' ? failed('certificate', certName, certificates.error) : pending('certificate', certName))
  }

  const dnsName = t('settings.developer.diag.dns', 'SN DNS')
  if (overview.data) {
    const sn = overview.data.sn
    result.push(
      !sn.host
        ? { key: 'dns', name: dnsName, status: 'pass', message: t('settings.developer.diag.dnsNoSn', 'This Zone does not use an SN.') }
        : sn.dnsARecords.length > 0
          ? { key: 'dns', name: dnsName, status: 'pass', message: t('settings.developer.diag.dnsOk', '{{domain}} resolves to {{records}} via {{sn}}.', { domain: overview.data.zone.domain, records: sn.dnsARecords.join(', '), sn: sn.host }) }
          : { key: 'dns', name: dnsName, status: 'warn', message: sn.digError || t('settings.developer.diag.dnsMissing', 'The SN returned no A record for {{domain}}.', { domain: overview.data.zone.domain }) },
    )
  } else {
    result.push(overview.status === 'error' ? failed('dns', dnsName, overview.error) : pending('dns', dnsName))
  }
  return result
}

function DiagnosticsSection() {
  const { t } = useI18n()
  const store = useSettingsStore()
  const devices = useSettingsResource('devices')
  const certificates = useSettingsResource('certificates')
  const overview = useSettingsResource('zoneOverview')
  const items = buildDiagnostics(devices, certificates, overview, t)
  const busy = [devices, certificates, overview].some((resource) => resource.status === 'loading')

  return (
    <Section
      title={t('settings.developer.diagnostics', 'System Diagnostics')}
      testId="settings-dev-diagnostics"
      actions={
        <Tooltip title={t('settings.common.refresh', 'Refresh')}>
          <span>
            <IconButton
              size="small"
              aria-label={t('settings.common.refresh', 'Refresh')}
              disabled={busy}
              onClick={() => {
                void store.load('devices', true)
                void store.load('certificates', true)
                void store.load('zoneOverview', true)
              }}
            >
              <RefreshCw size={14} />
            </IconButton>
          </span>
        </Tooltip>
      }
    >
      <div className="space-y-1.5">
        {items.map((item) => {
          const Icon = diagnosticIcons[item.status]
          return (
            <div
              key={item.key}
              className="flex items-center gap-3 rounded-lg p-2.5"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <Icon size={16} className={item.status === 'pending' ? 'animate-spin' : undefined} style={{ color: diagnosticColors[item.status] }} />
              <div className="min-w-0 flex-1">
                <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{item.name}</span>
                <p className="mt-0.5 break-words text-xs" style={{ color: 'var(--cp-muted)' }}>{item.message}</p>
              </div>
            </div>
          )
        })}
      </div>
    </Section>
  )
}

function ConfigExplorerSection() {
  const { t, locale } = useI18n()
  const [files, setFiles] = useState<ConfigFileSummary[] | null>(null)
  const [etcDir, setEtcDir] = useState('')
  const [listError, setListError] = useState<string | null>(null)
  const [selected, setSelected] = useState<string | null>(null)
  const [file, setFile] = useState<GatewayFileResponse | null>(null)
  const [fileError, setFileError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    void fetchGatewayOverview().then(({ data, error }) => {
      if (cancelled) return
      if (!data) {
        setListError(error instanceof Error ? error.message : String(error ?? ''))
        return
      }
      setEtcDir(data.etcDir)
      setFiles(data.files)
    })
    return () => { cancelled = true }
  }, [])

  const open = (name: string) => {
    setSelected(name)
    setFile(null)
    setFileError(null)
    void fetchGatewayFile(name).then(({ data, error }) => {
      if (data) setFile(data)
      else setFileError(error instanceof Error ? error.message : String(error ?? ''))
    })
  }

  return (
    <Section title={t('settings.developer.configExplorer', 'Config Explorer')} testId="settings-dev-config">
      {listError ? (
        <SectionNote tone="danger">{t('settings.developer.configUnavailable', 'Gateway configuration could not be read: {{detail}}', { detail: listError })}</SectionNote>
      ) : !files ? (
        <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>
      ) : (
        <div className="flex flex-col overflow-hidden rounded-lg sm:flex-row" style={{ border: '1px solid var(--cp-border)', minHeight: 220 }}>
          <div className="shrink-0 p-2 sm:w-52" style={{ borderBottom: '1px solid var(--cp-border)' }}>
            <p className="mb-1 px-1 font-mono text-[11px]" style={{ color: 'var(--cp-muted)' }}>{etcDir}</p>
            {files.map((item) => (
              <button
                key={item.name}
                type="button"
                disabled={!item.exists}
                onClick={() => open(item.name)}
                className="flex w-full items-center gap-1.5 rounded px-1 py-1 text-left text-xs transition-colors disabled:opacity-50"
                style={{
                  color: selected === item.name ? 'var(--cp-accent)' : 'var(--cp-text)',
                  background: selected === item.name ? 'color-mix(in srgb, var(--cp-accent) 10%, transparent)' : 'transparent',
                }}
              >
                <FileText size={12} className="shrink-0" style={{ color: 'var(--cp-muted)' }} />
                <span className="truncate">{item.name}</span>
              </button>
            ))}
          </div>
          <div className="min-w-0 flex-1 overflow-auto p-3" style={{ borderLeft: '1px solid var(--cp-border)' }}>
            {file ? (
              <>
                <p className="mb-2 text-[11px]" style={{ color: 'var(--cp-muted)' }}>
                  {file.path} · {formatBytes(file.sizeBytes)} · {formatDateTime(file.modifiedAt, locale)}
                </p>
                <pre className="whitespace-pre-wrap break-all text-xs" style={{ color: 'var(--cp-text)' }}>{file.content}</pre>
              </>
            ) : fileError ? (
              <SectionNote tone="danger">{fileError}</SectionNote>
            ) : (
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                {selected ? t('settings.common.loading', 'Loading…') : t('settings.developer.selectConfig', 'Select a configuration file to view its contents.')}
              </p>
            )}
          </div>
        </div>
      )}
    </Section>
  )
}

const logRanges = [
  { hours: 1, key: 'settings.developer.logRange.hour', label: 'Last hour' },
  { hours: 24, key: 'settings.developer.logRange.day', label: 'Last 24 hours' },
  { hours: 24 * 7, key: 'settings.developer.logRange.week', label: 'Last 7 days' },
]

function LogsSection() {
  const { t } = useI18n()
  const [services, setServices] = useState<string[] | null>(null)
  const [hours, setHours] = useState(24)
  const [state, setState] = useState<{ kind: 'idle' | 'working' } | { kind: 'failed'; detail: string }>({ kind: 'idle' })

  useEffect(() => {
    let cancelled = false
    void fetchLogServices().then(({ data, error }) => {
      if (cancelled) return
      if (data) setServices(data.services.map((service) => service.id))
      else setState({ kind: 'failed', detail: error instanceof Error ? error.message : String(error ?? '') })
    })
    return () => { cancelled = true }
  }, [])

  const download = async () => {
    if (!services?.length) return
    setState({ kind: 'working' })
    const since = new Date(Date.now() - hours * 3_600_000).toISOString()
    const { data, error } = await requestLogDownload(services, since)
    if (!data) {
      setState({ kind: 'failed', detail: error instanceof Error ? error.message : String(error ?? '') })
      return
    }
    downloadUrl(data.url, data.filename)
    setState({ kind: 'idle' })
  }

  return (
    <Section
      title={t('settings.developer.logs', 'Logs')}
      description={t('settings.developer.logsDesc', 'Download the logs of every system service as a zip for troubleshooting.')}
      testId="settings-dev-logs"
    >
      <div className="flex flex-wrap items-center gap-3">
        <TextField
          select
          size="small"
          value={hours}
          onChange={(event) => setHours(Number(event.target.value))}
          SelectProps={{ native: true }}
          inputProps={{ 'aria-label': t('settings.developer.logRange', 'Time range') }}
        >
          {logRanges.map((range) => (
            <option key={range.hours} value={range.hours}>{t(range.key, range.label)}</option>
          ))}
        </TextField>
        <Button
          size="small"
          variant="outlined"
          startIcon={<Download size={14} />}
          onClick={() => void download()}
          disabled={!services?.length || state.kind === 'working'}
          data-testid="settings-download-logs"
        >
          {state.kind === 'working' ? t('settings.developer.exporting', 'Packing…') : t('settings.developer.downloadLogs', 'Download Logs')}
        </Button>
        {services ? (
          <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('settings.developer.logServices', '{{count}} services', { count: services.length })}
          </span>
        ) : null}
      </div>
      {state.kind === 'failed' ? (
        <p className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>{t('settings.developer.logsFailed', 'Logs could not be packed: {{detail}}', { detail: state.detail })}</p>
      ) : null}
    </Section>
  )
}

const BUCKYCLI = '/opt/buckyos/bin/buckycli/buckycli'

const cliCommands = [
  { command: `${BUCKYCLI} version`, key: 'settings.developer.cli.version', description: 'Show the buckycli version' },
  { command: `${BUCKYCLI} node check`, key: 'settings.developer.cli.check', description: 'Read-only check of the local BuckyOS runtime' },
  { command: `sudo ${BUCKYCLI} node restart`, key: 'settings.developer.cli.restart', description: 'Restart the local BuckyOS runtime' },
  { command: `${BUCKYCLI} sys_config --list devices`, key: 'settings.developer.cli.devices', description: 'List the devices registered in system config' },
  { command: `${BUCKYCLI} sys_config --get boot/config`, key: 'settings.developer.cli.bootConfig', description: 'Show the Zone boot configuration' },
  { command: 'ls /opt/buckyos/logs', key: 'settings.developer.cli.logs', description: 'Log folders of the system services on this node' },
]

function CliSection() {
  const { t } = useI18n()
  return (
    <Section
      title={t('settings.developer.cliHelpers', 'CLI Helpers')}
      description={t('settings.developer.cliHelpersDesc', 'Run these on a node of the Zone.')}
    >
      <div className="space-y-1">
        {cliCommands.map((item) => (
          <ListCard key={item.command}>
            <div className="flex items-start gap-3">
              <Terminal size={14} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-accent)' }} />
              <div className="min-w-0 flex-1">
                <code className="break-all font-mono text-xs font-semibold" style={{ color: 'var(--cp-text)' }}>{item.command}</code>
                <p className="mt-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>{t(item.key, item.description)}</p>
              </div>
              <CopyButton label={t('settings.common.copy', 'Copy')} text={() => item.command} />
            </div>
          </ListCard>
        ))}
      </div>
    </Section>
  )
}

export function DeveloperModePage() {
  const { t, locale } = useI18n()
  const developer = useSettingsResource('developer')
  const store = useSettingsStore()
  const requestSudo = useSudoByPassword()
  const saving = useSettingsSnapshot().developerSaving
  const [actionError, setActionError] = useState<string | null>(null)
  const config = developer.data
  const enabled = config?.enabled ?? false

  const handleDeveloperModeChange = async () => {
    const next = !enabled
    setActionError(null)
    if (isMockRuntime()) {
      await store.setDeveloperMode(next, null)
      return
    }

    try {
      const grant = await requestSudo({
        aud: 'system-config',
        title: next
          ? t('settings.developer.enableDialogTitle', 'Enable Developer Mode')
          : t('settings.developer.disableDialogTitle', 'Disable Developer Mode'),
        description: t(
          'settings.developer.sudoDescription',
          'Confirm your administrator password to change the system developer mode.',
        ),
        reason: next
          ? t('settings.developer.enableReason', 'Enable system developer capabilities.')
          : t('settings.developer.disableReason', 'Disable system developer capabilities.'),
        confirmLabel: next
          ? t('settings.developer.enableConfirm', 'Enable')
          : t('settings.developer.disableConfirm', 'Disable'),
      })
      if (grant) {
        await store.setDeveloperMode(next, grant.sessionToken)
      }
    } catch {
      setActionError(t('settings.developer.sudoFailed', 'Unable to request administrator permission.'))
    }
  }

  return (
    <div className="space-y-4" data-testid="settings-developer-page">
      <Section title={t('settings.developer.modeSwitch', 'Mode')}>
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {t('settings.developer.developerMode', 'Developer Mode')}
            </p>
            <p className="mt-0.5 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('settings.developer.requiresSudo', 'Changing this system setting requires administrator permission.')}
            </p>
          </div>
          <Switch
            checked={enabled}
            disabled={!config || developer.status === 'loading' || saving}
            inputProps={{ 'aria-label': t('settings.developer.developerMode', 'Developer Mode') }}
            onChange={() => void handleDeveloperModeChange()}
          />
        </div>
        {developer.status === 'loading' && !config ? (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('settings.developer.loading', 'Loading developer mode configuration…')}</p>
        ) : null}
        {saving ? (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{t('settings.developer.saving', 'Saving developer mode configuration…')}</p>
        ) : null}
        {developer.status === 'error' || actionError ? (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>
            {actionError ?? t('settings.developer.unavailable', 'Developer mode configuration is unavailable.')}
          </p>
        ) : null}
        {config?.enabled_at || config?.enabled_by ? (
          <div className="mt-2 space-y-0.5">
            {config.enabled_at ? (
              <InfoRow label={t('settings.developer.lastEnabledAt', 'Last Enabled')} value={formatDateTime(config.enabled_at, locale)} />
            ) : null}
            {config.enabled_by ? (
              <InfoRow label={t('settings.developer.enabledBy', 'Enabled By')} value={config.enabled_by} />
            ) : null}
          </div>
        ) : null}
      </Section>

      {enabled ? (
        <>
          <DiagnosticsSection />
          <ConfigExplorerSection />
          <LogsSection />
          <CliSection />
        </>
      ) : config ? (
        <SectionNote>{t('settings.developer.offHint', 'Turn on developer mode to see diagnostics, configuration and logs.')}</SectionNote>
      ) : null}
    </div>
  )
}
