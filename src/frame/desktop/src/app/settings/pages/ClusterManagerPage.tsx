import { ChevronRight, Globe, Plus, RefreshCw, Server, ShieldCheck, Wifi } from 'lucide-react'
import { Alert, IconButton, Snackbar, Tooltip } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import type {
  ZoneCertificate,
  ZoneCertificatesResponse,
  ZoneDevice,
  ZoneDevicesResponse,
} from '../../../api/settings'
import { useZonePresentation } from '../../../presentation/zone'
import { useSettingsResource, useSettingsStore } from '../hooks/use-settings-store'
import { CollapsibleSection, InfoRow, ListCard, Section, SectionNote, StatusBadge } from '../components/shared/Section'
import { CopyButton } from '../components/shared/CopyButton'
import { formatBytes, formatDate, formatDateTime, formatRelative, formatUsage } from '../components/shared/format'

type Translate = ReturnType<typeof useI18n>['t']

function roleLabel(device: ZoneDevice, t: Translate): string {
  switch (device.role) {
    case 'ood':
      return t('settings.cluster.role.ood', 'OOD')
    case 'gateway':
      return t('settings.cluster.role.gateway', 'Gateway')
    case 'device':
      return t('settings.cluster.role.device', 'Device')
    default:
      return device.role
  }
}

function didMethod(did: string): string {
  const parts = did.split(':')
  return parts.length >= 3 ? `${parts[0]}:${parts[1]}` : '—'
}

function osText(device: ZoneDevice): string {
  const os = device.os_version || device.os || ''
  return [os, device.arch].filter(Boolean).join(' · ') || '—'
}

function cpuText(device: ZoneDevice, t: Translate): string {
  const cpu = device.cpu
  if (!cpu?.model && !cpu?.cores) return '—'
  const cores = cpu.cores ? t('settings.cluster.cores', '{{count}} cores', { count: cpu.cores }) : null
  return [cpu.model, cores].filter(Boolean).join(' · ')
}

function ipv6Status(state: string | null | undefined): 'pass' | 'warn' | 'neutral' {
  if (!state) return 'neutral'
  return state === 'available' ? 'pass' : 'warn'
}

function certificateStatus(cert: ZoneCertificate): 'pass' | 'warn' | 'fail' {
  if (cert.state === 'expired' || cert.state === 'unusable') return 'fail'
  if (cert.state !== 'valid') return 'warn'
  return cert.days_remaining != null && cert.days_remaining < 14 ? 'warn' : 'pass'
}

function certificateStateLabel(cert: ZoneCertificate, t: Translate): string {
  switch (cert.state) {
    case 'valid':
      return t('settings.cluster.certValid', 'Valid')
    case 'expired':
      return t('settings.cluster.certExpired', 'Expired')
    case 'not_yet_valid':
      return t('settings.cluster.certNotYetValid', 'Not yet valid')
    case 'unusable':
      return t('settings.cluster.certUnusable', 'Unusable')
    default:
      return t('settings.cluster.certUnknown', 'Unknown')
  }
}

function clusterInfoJson(zone: ZoneDevicesResponse | null, certs: ZoneCertificatesResponse | null): string {
  return JSON.stringify(
    {
      zone_did: zone?.zone_did ?? null,
      owner: zone?.owner_did ?? null,
      hostname: zone?.hostname ?? null,
      sn: zone?.sn ?? null,
      nodes: (zone?.devices ?? []).map((device) => ({
        name: device.name,
        did: device.did ?? null,
        role: device.role,
        online: device.online,
        last_seen: device.last_seen ?? null,
        net_id: device.net_id ?? null,
        ips: device.ips,
        ipv6: device.ipv6_state ?? null,
      })),
      certificates: (certs?.certificates ?? []).map((cert) => ({
        host: cert.host,
        source: cert.source,
        issuer: cert.issuer ?? null,
        not_after: cert.not_after ?? null,
        state: cert.state,
      })),
    },
    null,
    2,
  )
}

function DeviceRow({ device }: { device: ZoneDevice }) {
  const { t, locale } = useI18n()
  const [open, setOpen] = useState(false)
  const subtitle = [device.hostname, osText(device), device.ips[0]].filter((part) => part && part !== '—').join(' · ')

  return (
    <ListCard testId={`settings-device-${device.name}`}>
      <button
        type="button"
        className="flex w-full items-center gap-3 text-left"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
      >
        <ChevronRight
          size={14}
          className="shrink-0 transition-transform"
          style={{ color: 'var(--cp-muted)', transform: open ? 'rotate(90deg)' : undefined }}
        />
        <Server size={16} className="shrink-0" style={{ color: 'var(--cp-muted)' }} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{device.name}</span>
            <StatusBadge status="info" label={roleLabel(device, t)} />
            {device.is_local ? <StatusBadge status="neutral" label={t('settings.cluster.thisNode', 'This node')} /> : null}
          </div>
          {subtitle ? (
            <p className="mt-0.5 truncate text-xs" style={{ color: 'var(--cp-muted)' }}>{subtitle}</p>
          ) : null}
        </div>
        <StatusBadge
          status={device.online ? 'pass' : 'fail'}
          label={device.online ? t('settings.cluster.online', 'Online') : t('settings.cluster.offline', 'Offline')}
        />
      </button>

      {open ? (
        <div className="mt-2 border-t pt-2" style={{ borderColor: 'var(--cp-border)' }} data-testid={`settings-device-detail-${device.name}`}>
          {device.info_error ? (
            <SectionNote tone="warning">{t('settings.cluster.deviceNotReported', 'This device has not reported its details yet.')}</SectionNote>
          ) : null}
          <div className="space-y-0.5">
            {device.did ? (
              <InfoRow label={t('settings.cluster.deviceDid', 'Device DID')} value={<span className="break-all font-mono text-xs">{device.did}</span>} />
            ) : null}
            {device.hostname ? <InfoRow label={t('settings.cluster.hostname', 'Hostname')} value={device.hostname} /> : null}
            <InfoRow label={t('settings.general.os', 'Operating System')} value={osText(device)} />
            <InfoRow label={t('settings.general.cpu', 'CPU')} value={cpuText(device, t)} />
            {device.cpu?.usage_percent != null ? (
              <InfoRow label={t('settings.cluster.cpuUsage', 'CPU Usage')} value={`${Math.round(device.cpu.usage_percent)}%`} />
            ) : null}
            <InfoRow label={t('settings.general.memory', 'Memory')} value={formatUsage(device.memory?.used_bytes, device.memory?.total_bytes)} />
            <InfoRow label={t('settings.general.storage', 'Storage')} value={formatUsage(device.storage?.used_bytes, device.storage?.total_bytes)} />
            {device.gpu ? (
              <InfoRow
                label={t('settings.cluster.gpu', 'GPU')}
                value={[device.gpu.model, device.gpu.total_mem_bytes ? formatBytes(device.gpu.total_mem_bytes) : null, device.gpu.tflops ? `${device.gpu.tflops} TFLOPS` : null].filter(Boolean).join(' · ')}
              />
            ) : null}
            <InfoRow label={t('settings.cluster.network', 'Network')} value={device.net_id || '—'} />
            <InfoRow
              label={t('settings.cluster.ipAddresses', 'IP Addresses')}
              value={device.ips.length ? <span className="font-mono text-xs">{device.ips.join(', ')}</span> : '—'}
            />
            {device.ipv6_state ? (
              <InfoRow label={t('settings.cluster.ipv6', 'IPv6')} value={<StatusBadge status={ipv6Status(device.ipv6_state)} label={device.ipv6_state} />} />
            ) : null}
            <InfoRow
              label={t('settings.cluster.lastSeen', 'Last Report')}
              value={device.last_seen ? `${formatDateTime(device.last_seen, locale)} (${formatRelative(device.last_seen, locale)})` : '—'}
            />
          </div>
        </div>
      ) : null}
    </ListCard>
  )
}

function DevicesSection({ zone, status }: { zone: ZoneDevicesResponse | null; status: string }) {
  const { t } = useI18n()
  const [notice, setNotice] = useState(false)

  return (
    <Section
      title={t('settings.cluster.nodesDevices', 'Nodes / Devices')}
      testId="settings-cluster-devices"
      actions={
        <Tooltip title={t('settings.cluster.addDevice', 'Add device')}>
          <IconButton size="small" aria-label={t('settings.cluster.addDevice', 'Add device')} onClick={() => setNotice(true)} data-testid="settings-add-device">
            <Plus size={16} />
          </IconButton>
        </Tooltip>
      }
    >
      {zone ? (
        zone.devices.length > 0 ? (
          <div className="space-y-2">
            {zone.devices.map((device) => <DeviceRow key={device.name} device={device} />)}
          </div>
        ) : (
          <SectionNote>{t('settings.cluster.noDevices', 'No device has registered in this Zone yet.')}</SectionNote>
        )
      ) : status === 'error' ? (
        <SectionNote tone="danger">{t('settings.cluster.devicesUnavailable', 'The device list could not be loaded.')}</SectionNote>
      ) : (
        <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>
      )}
      <Snackbar
        open={notice}
        autoHideDuration={3000}
        onClose={() => setNotice(false)}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert severity="info" variant="filled" onClose={() => setNotice(false)} data-testid="settings-add-device-notice">
          {t('settings.cluster.addDeviceUnsupported', 'Adding devices is not supported in BuckyOS Desktop yet.')}
        </Alert>
      </Snackbar>
    </Section>
  )
}

function ConnectivitySection({ localDevice }: { localDevice: ZoneDevice | null }) {
  const { t } = useI18n()
  const store = useSettingsStore()
  const overview = useSettingsResource('zoneOverview')
  const sn = overview.data?.sn
  const publicEndpoints = (localDevice?.endpoints ?? []).filter((endpoint) => endpoint.scope === 'global' && endpoint.ip)

  return (
    <Section
      title={t('settings.cluster.connectivity', 'Connectivity')}
      testId="settings-cluster-connectivity"
      actions={
        <Tooltip title={t('settings.common.refresh', 'Refresh')}>
          <span>
            <IconButton size="small" aria-label={t('settings.common.refresh', 'Refresh')} disabled={overview.status === 'loading'} onClick={() => void store.load('zoneOverview', true)}>
              <RefreshCw size={14} />
            </IconButton>
          </span>
        </Tooltip>
      }
    >
      {overview.data && sn ? (
        <div className="space-y-0.5">
          <InfoRow label={t('settings.cluster.domain', 'Domain')} value={<span className="break-all font-mono text-xs">{overview.data.zone.domain || '—'}</span>} />
          <InfoRow
            label={t('settings.cluster.snServer', 'SN Server')}
            value={sn.host ? <span className="font-mono text-xs">{sn.host}</span> : t('settings.cluster.snNotUsed', 'Not used')}
          />
          {sn.host ? <InfoRow label={t('settings.cluster.snAddress', 'SN Address')} value={<span className="font-mono text-xs">{sn.ip || '—'}</span>} /> : null}
          {sn.host ? (
            <InfoRow
              label={t('settings.cluster.dnsViaSn', 'Zone DNS via SN')}
              value={
                sn.dnsARecords.length > 0
                  ? <span className="font-mono text-xs">{sn.dnsARecords.join(', ')}</span>
                  : <StatusBadge status="warn" label={t('settings.cluster.noRecord', 'No record')} />
              }
            />
          ) : null}
          {localDevice ? (
            <>
              <InfoRow label={t('settings.cluster.nodeNetwork', 'Network of {{node}}', { node: localDevice.name })} value={localDevice.net_id || '—'} />
              <InfoRow
                label={t('settings.cluster.ipv6', 'IPv6')}
                value={localDevice.ipv6_state ? <StatusBadge status={ipv6Status(localDevice.ipv6_state)} label={localDevice.ipv6_state} /> : '—'}
              />
              <InfoRow
                label={t('settings.cluster.publicAddresses', 'Public Addresses')}
                value={publicEndpoints.length ? <span className="font-mono text-xs">{publicEndpoints.map((endpoint) => endpoint.ip).join(', ')}</span> : '—'}
              />
            </>
          ) : null}
          {sn.digError ? <SectionNote tone="warning">{t('settings.cluster.dnsCheckFailed', 'DNS self-check failed: {{detail}}', { detail: sn.digError })}</SectionNote> : null}
          {sn.dnsTxtRecords.length > 0 ? (
            <CollapsibleSection title={t('settings.cluster.dnsDetails', 'DNS TXT Records')}>
              <pre className="whitespace-pre-wrap break-all font-mono text-xs" style={{ color: 'var(--cp-text)' }}>
                {sn.dnsTxtRecords.join('\n')}
              </pre>
            </CollapsibleSection>
          ) : null}
        </div>
      ) : overview.status === 'error' ? (
        <SectionNote tone="danger">{t('settings.cluster.connectivityUnavailable', 'Connectivity information could not be loaded.')}</SectionNote>
      ) : (
        <SectionNote>{t('settings.cluster.checkingConnectivity', 'Checking connectivity…')}</SectionNote>
      )}
    </Section>
  )
}

function CertificatesSection() {
  const { t, locale } = useI18n()
  const certs = useSettingsResource('certificates')
  const data = certs.data

  return (
    <Section title={t('settings.cluster.certificates', 'Certificates')} testId="settings-cluster-certificates">
      {data ? (
        data.certificates.length > 0 ? (
          <div className="space-y-2">
            {data.certificates.map((cert) => (
              <ListCard key={cert.host}>
                <div className="flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <ShieldCheck size={14} className="shrink-0" style={{ color: 'var(--cp-muted)' }} />
                    <span className="truncate font-mono text-xs font-medium" style={{ color: 'var(--cp-text)' }}>{cert.host}</span>
                  </div>
                  <StatusBadge status={certificateStatus(cert)} label={certificateStateLabel(cert, t)} />
                </div>
                <div className="mt-1 space-y-0.5">
                  <InfoRow
                    label={t('settings.cluster.certSource', 'Source')}
                    value={cert.source === 'acme' ? t('settings.cluster.certAcme', 'Issued and renewed automatically (ACME)') : t('settings.cluster.certCustom', 'Installed manually')}
                  />
                  <InfoRow label={t('settings.cluster.certIssuer', 'Issuer')} value={cert.issuer || '—'} />
                  <InfoRow
                    label={t('settings.cluster.certExpiry', 'Valid Until')}
                    value={
                      cert.days_remaining != null && cert.days_remaining >= 0
                        ? t('settings.cluster.certExpiryDays', '{{date}} ({{days}} days left)', { date: formatDate(cert.not_after, locale), days: cert.days_remaining })
                        : formatDate(cert.not_after, locale)
                    }
                  />
                  {cert.dns_names.length > 0 ? (
                    <InfoRow label={t('settings.cluster.certNames', 'Covers')} value={<span className="break-all font-mono text-xs">{cert.dns_names.join(', ')}</span>} />
                  ) : null}
                </div>
                {cert.cert_pem || cert.fingerprint_sha256 ? (
                  <CollapsibleSection title={t('settings.cluster.x509Details', 'X.509 Certificate')}>
                    {cert.fingerprint_sha256 ? (
                      <InfoRow label={t('settings.cluster.certFingerprint', 'SHA-256 Fingerprint')} value={<span className="break-all font-mono text-xs">{cert.fingerprint_sha256}</span>} />
                    ) : null}
                    {cert.cert_pem ? (
                      <pre className="mt-2 overflow-x-auto rounded-lg p-2 text-xs" style={{ color: 'var(--cp-text)', background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}>
                        {cert.cert_pem}
                      </pre>
                    ) : null}
                  </CollapsibleSection>
                ) : null}
              </ListCard>
            ))}
            {data.node ? (
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('settings.cluster.certReadOn', 'Read from node {{node}}.', { node: data.node })}</p>
            ) : null}
          </div>
        ) : (
          <SectionNote>{t('settings.cluster.noCertificates', 'No TLS certificate for {{host}} is installed on this node.', { host: data.hostname })}</SectionNote>
        )
      ) : certs.status === 'error' ? (
        <SectionNote tone="danger">{t('settings.cluster.certificatesUnavailable', 'Certificate information could not be loaded.')}</SectionNote>
      ) : (
        <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>
      )}
    </Section>
  )
}

export function ClusterManagerPage() {
  const { t } = useI18n()
  const devices = useSettingsResource('devices')
  const certificates = useSettingsResource('certificates')
  const zonePresentation = useZonePresentation()
  const zone = devices.data
  const nodes = zone?.devices ?? []
  const online = nodes.filter((device) => device.online).length
  const oods = nodes.filter((device) => device.is_ood).length
  const localDevice = nodes.find((device) => device.is_local) ?? null

  const tiles = [
    {
      label: t('settings.cluster.mode', 'Mode'),
      value: zone ? (oods > 1 ? t('settings.cluster.multiNode', 'Multi Node') : t('settings.cluster.singleNode', 'Single Node')) : '—',
      icon: Server,
    },
    { label: t('settings.cluster.nodes', 'Nodes'), value: zone ? `${online} / ${nodes.length}` : '—', icon: Server },
    { label: t('settings.cluster.domain', 'Domain'), value: zone?.hostname ?? '—', icon: Globe },
    {
      label: t('settings.cluster.access', 'Access'),
      value: zone ? (zone.sn ? t('settings.cluster.viaSn', 'Via SN') : t('settings.cluster.direct', 'Direct')) : '—',
      icon: Wifi,
    },
  ]

  return (
    <div className="space-y-4" data-testid="settings-cluster-page">
      <Section title={t('settings.cluster.overview', 'Cluster Overview')}>
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          {tiles.map((item) => (
            <div
              key={item.label}
              className="min-w-0 rounded-xl p-3 text-center"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <item.icon size={16} className="mx-auto mb-1.5" style={{ color: 'var(--cp-muted)' }} />
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{item.label}</p>
              <p className="mt-0.5 truncate text-sm font-semibold" style={{ color: 'var(--cp-text)' }} title={String(item.value)}>{item.value}</p>
            </div>
          ))}
        </div>
      </Section>

      <DevicesSection zone={zone} status={devices.status} />

      <Section title={t('settings.cluster.zoneTitle', 'Zone')}>
        {zone ? (
          <div className="space-y-0.5">
            <InfoRow label={t('settings.cluster.zoneName', 'Name')} value={zonePresentation.title} />
            <InfoRow label={t('settings.cluster.zoneDID', 'Zone DID')} value={<span className="break-all font-mono text-xs">{zone.zone_did}</span>} />
            <InfoRow label={t('settings.cluster.didMethod', 'DID Method')} value={didMethod(zone.zone_did)} />
            <InfoRow label={t('settings.cluster.ownerDID', 'Owner DID')} value={<span className="break-all font-mono text-xs">{zone.owner_did ?? '—'}</span>} />
          </div>
        ) : (
          <SectionNote tone={devices.status === 'error' ? 'danger' : 'muted'}>
            {devices.status === 'error' ? t('settings.cluster.devicesUnavailable', 'The device list could not be loaded.') : t('settings.common.loading', 'Loading…')}
          </SectionNote>
        )}
      </Section>

      <ConnectivitySection localDevice={localDevice} />

      <CertificatesSection />

      <Section title={t('settings.cluster.debugExport', 'Debug & Export')}>
        <CopyButton
          label={t('settings.cluster.copyClusterInfo', 'Copy Cluster Info')}
          text={() => clusterInfoJson(zone, certificates.data)}
          testId="settings-copy-cluster-info"
        />
      </Section>
    </div>
  )
}
