import { Download } from 'lucide-react'
import { Button } from '@mui/material'
import { useI18n } from '../../../i18n/provider'
import type { BuckyOSInfo, ZoneDevicesResponse } from '../../../api/settings'
import { useSettingsResource } from '../hooks/use-settings-store'
import { Section, InfoRow, SectionNote, StatusBadge } from '../components/shared/Section'
import { CopyButton } from '../components/shared/CopyButton'
import { downloadText, formatDateTime, formatUsage } from '../components/shared/format'

function releaseChannelStatus(channel: string): 'pass' | 'warn' | 'neutral' {
  const normalized = channel.trim().toLowerCase()
  if (normalized === 'stable') return 'pass'
  return normalized ? 'warn' : 'neutral'
}

function unixToIso(value: number | null | undefined): string | null {
  return value && value > 0 ? new Date(value * 1000).toISOString() : null
}

/** Totals over the nodes that reported their storage. */
function storageTotals(zone: ZoneDevicesResponse | null) {
  let used = 0
  let total = 0
  for (const device of zone?.devices ?? []) {
    if (device.storage?.total_bytes) {
      total += device.storage.total_bytes
      used += device.storage.used_bytes ?? 0
    }
  }
  return total > 0 ? { used, total } : null
}

function systemInfoJson(software: BuckyOSInfo | null, zone: ZoneDevicesResponse | null): string {
  return JSON.stringify(
    {
      buckyos: software
        ? {
            version: software.version,
            build: software.build_version ?? null,
            channel: software.release_channel,
            target: software.target,
            installed_at: unixToIso(software.installed_at),
            updated_at: unixToIso(software.updated_at),
          }
        : null,
      zone: zone ? { did: zone.zone_did, hostname: zone.hostname ?? null, sn: zone.sn ?? null } : null,
      nodes: (zone?.devices ?? []).map((device) => ({
        name: device.name,
        role: device.role,
        online: device.online,
        os: device.os_version ?? device.os ?? null,
        arch: device.arch ?? null,
        cpu: device.cpu?.model ?? null,
        cpu_cores: device.cpu?.cores ?? null,
        memory_total: device.memory?.total_bytes ?? null,
        storage_total: device.storage?.total_bytes ?? null,
        storage_used: device.storage?.used_bytes ?? null,
      })),
      generated_at: new Date().toISOString(),
    },
    null,
    2,
  )
}

export function GeneralPage() {
  const { t, locale } = useI18n()
  const software = useSettingsResource('software')
  const devices = useSettingsResource('devices')
  const info = software.data
  const zone = devices.data
  const nodes = zone?.devices ?? []
  const online = nodes.filter((device) => device.online).length
  const storage = storageTotals(zone)

  return (
    <div className="space-y-4" data-testid="settings-general-page">
      <Section title={t('settings.general.softwareInfo', 'Software Info')} testId="settings-software-info">
        {info ? (
          <div className="space-y-0.5">
            <InfoRow label={t('settings.general.version', 'BuckyOS Version')} value={info.version} />
            <InfoRow label={t('settings.general.buildVersion', 'Build Version')} value={info.build_version?.trim() || '—'} />
            <InfoRow
              label={t('settings.general.releaseChannel', 'Release Channel')}
              value={<StatusBadge status={releaseChannelStatus(info.release_channel)} label={info.release_channel || '—'} />}
            />
            <InfoRow label={t('settings.general.target', 'Platform')} value={<span className="font-mono text-xs">{info.target}</span>} />
            <InfoRow label={t('settings.general.installedAt', 'Installed')} value={formatDateTime(info.installed_at, locale)} />
            <InfoRow label={t('settings.general.lastUpdate', 'Last Update')} value={formatDateTime(info.updated_at, locale)} />
          </div>
        ) : software.status === 'error' ? (
          <SectionNote tone="danger">{t('settings.general.softwareInfoUnavailable', 'System information is unavailable.')}</SectionNote>
        ) : (
          <SectionNote>{t('settings.general.loadingSoftwareInfo', 'Loading system information…')}</SectionNote>
        )}
      </Section>

      <Section title={t('settings.general.systemSnapshot', 'System Snapshot')} testId="settings-system-snapshot">
        {zone ? (
          <div className="space-y-0.5">
            <InfoRow label={t('settings.general.zoneDomain', 'Zone Domain')} value={<span className="break-all font-mono text-xs">{zone.hostname ?? '—'}</span>} />
            <InfoRow
              label={t('settings.general.nodeCount', 'Nodes')}
              value={t('settings.general.nodesOnline', '{{online}} of {{total}} online', { online: String(online), total: String(nodes.length) })}
            />
            <InfoRow
              label={t('settings.general.storageUsage', 'Storage Usage')}
              value={storage ? formatUsage(storage.used, storage.total) : '—'}
            />
          </div>
        ) : devices.status === 'error' ? (
          <SectionNote tone="danger">{t('settings.cluster.devicesUnavailable', 'The device list could not be loaded.')}</SectionNote>
        ) : (
          <SectionNote>{t('settings.common.loading', 'Loading…')}</SectionNote>
        )}
      </Section>

      <Section
        title={t('settings.general.support', 'Support & Debug')}
        description={t('settings.general.supportDesc', 'Copy or export system information for troubleshooting.')}
      >
        <div className="flex flex-wrap gap-2">
          <CopyButton
            label={t('settings.general.copySystemInfo', 'Copy System Info')}
            text={() => systemInfoJson(info, zone)}
            testId="settings-copy-system-info"
          />
          <Button
            size="small"
            variant="outlined"
            startIcon={<Download size={14} />}
            onClick={() => downloadText(`buckyos-system-info-${new Date().toISOString().slice(0, 10)}.json`, systemInfoJson(info, zone))}
            data-testid="settings-export-system-info"
          >
            {t('settings.general.exportJSON', 'Export as JSON')}
          </Button>
        </div>
      </Section>
    </div>
  )
}
