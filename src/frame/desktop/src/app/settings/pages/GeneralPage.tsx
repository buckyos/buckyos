import { Copy, Download, RefreshCw } from 'lucide-react'
import { Button } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'
import { Section, InfoRow } from '../components/shared/Section'
import { SettingsPageIntro } from '../components/shared/SettingsPageIntro'

export function GeneralPage() {
  const { t } = useI18n()
  const { general } = useSettingsSnapshot()
  const store = useSettingsStore()
  const { software, device, snapshot } = general
  const [copied, setCopied] = useState(false)

  const handleCopySystemInfo = () => {
    navigator.clipboard.writeText(store.getSystemInfoJSON())
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <div className="space-y-4">
      <SettingsPageIntro
        page="general"
        title={t('settings.general.title', 'General')}
        description={t(
          'settings.general.description',
          'System information, device details, and support tools.',
        )}
      />

      <Section title={t('settings.general.softwareInfo', 'Software Info')}>
        <div className="space-y-0.5">
          <InfoRow label={t('settings.general.version', 'BuckyOS Version')} value={software.version} />
          <InfoRow label={t('settings.general.buildVersion', 'Build Version')} value={software.buildVersion} />
          <InfoRow
            label={t('settings.general.releaseChannel', 'Release Channel')}
            value={
              <span
                className="inline-block px-2 py-0.5 rounded-full text-xs font-medium"
                style={{
                  color: software.releaseChannel === 'stable' ? 'var(--cp-success)' : 'var(--cp-warning)',
                  background: software.releaseChannel === 'stable'
                    ? 'color-mix(in srgb, var(--cp-success) 14%, transparent)'
                    : 'color-mix(in srgb, var(--cp-warning) 14%, transparent)',
                }}
              >
                {software.releaseChannel.charAt(0).toUpperCase() + software.releaseChannel.slice(1)}
              </span>
            }
          />
          {software.lastUpdateTime && (
            <InfoRow
              label={t('settings.general.lastUpdate', 'Last Update')}
              value={new Date(software.lastUpdateTime).toLocaleDateString()}
            />
          )}
        </div>
        {software.updateAvailable && (
          <div
            className="mt-3 flex items-center justify-between rounded-lg px-3 py-2"
            style={{
              background: 'color-mix(in srgb, var(--cp-accent) 10%, transparent)',
            }}
          >
            <span className="text-sm" style={{ color: 'var(--cp-accent)' }}>
              {t('settings.general.updateAvailable', 'New version available: {{version}}', { version: software.latestVersion ?? '' })}
            </span>
            <Button size="small" startIcon={<RefreshCw size={14} />}>
              {t('settings.general.updateNow', 'Update Now')}
            </Button>
          </div>
        )}
      </Section>

      {/* Device Info */}
      <Section title={t('settings.general.deviceInfo', 'Device Info')}>
        <div className="space-y-0.5">
          <InfoRow label={t('settings.general.os', 'Operating System')} value={`${device.osType} ${device.osVersion}`} />
          <InfoRow label={t('settings.general.cpu', 'CPU')} value={`${device.cpuModel} (${device.cpuCores} cores)`} />
          <InfoRow label={t('settings.general.memory', 'Memory')} value={device.totalMemory} />
          <InfoRow label={t('settings.general.storage', 'Storage')} value={device.totalStorage} />
        </div>
      </Section>

      {/* System Snapshot */}
      <Section title={t('settings.general.systemSnapshot', 'System Snapshot')}>
        <div className="space-y-0.5">
          <InfoRow
            label={t('settings.general.installMode', 'Install Mode')}
            value={snapshot.installMode.charAt(0).toUpperCase() + snapshot.installMode.slice(1)}
          />
          <InfoRow label={t('settings.general.nodeCount', 'Node Count')} value={snapshot.nodeCount} />
          <InfoRow
            label={t('settings.general.storageUsage', 'Storage Usage')}
            value={`${snapshot.storageUsed} / ${snapshot.storageTotal}`}
          />
        </div>
        {snapshot.enabledModules.length > 0 && (
          <div className="mt-3">
            <p className="text-xs mb-2" style={{ color: 'var(--cp-muted)' }}>
              {t('settings.general.enabledModules', 'Enabled Modules')}
            </p>
            <div className="flex flex-wrap gap-1.5">
              {snapshot.enabledModules.map((mod) => (
                <span
                  key={mod}
                  className="px-2 py-0.5 rounded-full text-xs"
                  style={{
                    color: 'var(--cp-text)',
                    background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)',
                  }}
                >
                  {mod}
                </span>
              ))}
            </div>
          </div>
        )}
      </Section>

      {/* Support / Debug */}
      <Section
        title={t('settings.general.support', 'Support & Debug')}
        description={t('settings.general.supportDesc', 'Copy system information for troubleshooting.')}
      >
        <div className="flex flex-wrap gap-2">
          <Button
            size="small"
            variant="outlined"
            startIcon={<Copy size={14} />}
            onClick={handleCopySystemInfo}
          >
            {copied
              ? t('settings.general.copied', 'Copied!')
              : t('settings.general.copySystemInfo', 'Copy System Info')}
          </Button>
          <Button
            size="small"
            variant="outlined"
            startIcon={<Download size={14} />}
          >
            {t('settings.general.exportJSON', 'Export as JSON')}
          </Button>
        </div>
      </Section>
    </div>
  )
}
