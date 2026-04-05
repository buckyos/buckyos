import { Globe, Users, Lock, MessageCircle, Shield, Cpu } from 'lucide-react'
import { useI18n } from '../../../i18n/provider'
import { useSettingsSnapshot } from '../hooks/use-settings-store'
import { Section, StatusBadge } from '../components/shared/Section'
import { SettingsPageIntro } from '../components/shared/SettingsPageIntro'
import type { DataVisibility, AppRiskLevel, DeviceAccessType } from '../mock/types'

const visibilityIcons: Record<string, typeof Globe> = {
  Globe,
  Users,
  Lock,
}

const riskLevelLabels: Record<AppRiskLevel, string> = {
  trusted: 'Trusted',
  elevated: 'Elevated',
  high_risk: 'High Risk',
}

export function PrivacyPage() {
  const { t } = useI18n()
  const { privacy } = useSettingsSnapshot()

  return (
    <div className="space-y-4">
      <SettingsPageIntro
        page="privacy"
        title={t('settings.privacy.title', 'Privacy')}
        description={t(
          'settings.privacy.description',
          'Understand who can access your system, data, and devices.',
        )}
      />

      <Section title={t('settings.privacy.publicAccess', 'Public Access')}>
        <div className="space-y-2">
          {privacy.publicAccess.map((entry) => (
            <div
              key={entry.domain}
              className="flex items-start gap-3 rounded-lg p-3"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <Globe
                size={16}
                className="mt-0.5 shrink-0"
                style={{ color: entry.isPublic ? 'var(--cp-warning)' : 'var(--cp-muted)' }}
              />
              <div className="flex-1 min-w-0">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {entry.label}
                  </span>
                  {entry.isPublic && (
                    <StatusBadge status="warn" label={t('settings.privacy.public', 'Public')} />
                  )}
                  {!entry.enabled && (
                    <span className="text-xs px-1.5 py-0.5 rounded" style={{ color: 'var(--cp-muted)', background: 'color-mix(in srgb, var(--cp-surface) 60%, transparent)' }}>
                      {t('settings.privacy.comingSoon', 'Coming Soon')}
                    </span>
                  )}
                </div>
                <p className="text-xs mt-1 font-mono" style={{ color: 'var(--cp-muted)' }}>
                  {entry.domain}
                </p>
                <p className="text-xs mt-1" style={{ color: 'var(--cp-muted)' }}>
                  {entry.description}
                </p>
              </div>
            </div>
          ))}
        </div>
      </Section>

      {/* Messaging Access */}
      <Section title={t('settings.privacy.messagingAccess', 'Messaging Access')}>
        <div
          className="flex items-center justify-between rounded-lg p-3"
          style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
        >
          <div className="flex items-center gap-3">
            <MessageCircle size={16} style={{ color: 'var(--cp-accent)' }} />
            <div>
              <p className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                {t('settings.privacy.allowExternalMessages', 'Allow external messages')}
              </p>
              <p className="text-xs mt-0.5" style={{ color: 'var(--cp-muted)' }}>
                {privacy.messagingAccess.description}
              </p>
            </div>
          </div>
          <div
            className="shrink-0 w-10 h-6 rounded-full relative"
            style={{
              background: privacy.messagingAccess.enabled ? 'var(--cp-success)' : 'var(--cp-muted)',
              opacity: privacy.messagingAccess.canToggle ? 1 : 0.6,
              cursor: privacy.messagingAccess.canToggle ? 'pointer' : 'not-allowed',
            }}
          >
            <div
              className="absolute top-0.5 w-5 h-5 rounded-full bg-white transition-all"
              style={{ left: privacy.messagingAccess.enabled ? '18px' : '2px' }}
            />
          </div>
        </div>
        {!privacy.messagingAccess.canToggle && (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('settings.privacy.messagingLocked', 'This setting is required for core messaging features and cannot be disabled.')}
          </p>
        )}
      </Section>

      {/* Data Visibility */}
      <Section
        title={t('settings.privacy.dataVisibility', 'Data Visibility')}
        description={t('settings.privacy.dataVisibilityDesc', 'Understand how your data is categorized and who can see it.')}
      >
        <div className="space-y-2">
          {privacy.dataVisibility.map((entry) => {
            const Icon = visibilityIcons[entry.icon] ?? Lock
            const colorMap: Record<DataVisibility, string> = {
              public: 'var(--cp-warning)',
              shared: 'var(--cp-accent)',
              private: 'var(--cp-success)',
            }
            const color = colorMap[entry.visibility]

            return (
              <div
                key={entry.folderName}
                className="flex items-start gap-3 rounded-lg p-3"
                style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
              >
                <Icon size={16} className="mt-0.5 shrink-0" style={{ color }} />
                <div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                      {entry.folderName}
                    </span>
                    <StatusBadge
                      status={entry.visibility === 'public' ? 'warn' : entry.visibility === 'shared' ? 'elevated' : 'pass'}
                      label={entry.visibility.charAt(0).toUpperCase() + entry.visibility.slice(1)}
                    />
                  </div>
                  <p className="text-xs mt-1" style={{ color: 'var(--cp-muted)' }}>
                    {entry.description}
                  </p>
                </div>
              </div>
            )
          })}
        </div>
      </Section>

      {/* App & Agent Access */}
      <Section
        title={t('settings.privacy.appAgentAccess', 'App & Agent Access')}
        description={t('settings.privacy.appAgentAccessDesc', 'See which apps and agents can access your data.')}
      >
        <div className="space-y-2">
          {privacy.appAccess.map((app) => (
            <div
              key={app.id}
              className="rounded-lg p-3"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Shield size={14} style={{ color: 'var(--cp-muted)' }} />
                  <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                    {app.name}
                  </span>
                  <span className="text-xs px-1.5 py-0.5 rounded" style={{
                    color: 'var(--cp-muted)',
                    background: 'color-mix(in srgb, var(--cp-surface) 60%, transparent)',
                  }}>
                    {app.type === 'agent' ? 'Agent' : app.type === 'system' ? 'System' : '3rd Party'}
                  </span>
                </div>
                <StatusBadge status={app.riskLevel} label={riskLevelLabels[app.riskLevel]} />
              </div>
              <p className="text-xs mt-1.5" style={{ color: 'var(--cp-muted)' }}>
                {app.accessScope}
                {app.multiUser && (
                  <span style={{ color: 'var(--cp-warning)' }}>
                    {' '}· {t('settings.privacy.multiUserAccess', 'Multi-user access')}
                  </span>
                )}
              </p>
              {app.extraPermissions.length > 0 && (
                <div className="flex flex-wrap gap-1 mt-1.5">
                  {app.extraPermissions.map((perm) => (
                    <span
                      key={perm}
                      className="text-xs px-1.5 py-0.5 rounded"
                      style={{
                        color: 'var(--cp-warning)',
                        background: 'color-mix(in srgb, var(--cp-warning) 10%, transparent)',
                      }}
                    >
                      {perm}
                    </span>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      </Section>

      {/* Device & Capability Permissions */}
      <Section
        title={t('settings.privacy.devicePermissions', 'Device & Capability Permissions')}
        description={t('settings.privacy.devicePermissionsDesc', 'See which apps use device capabilities.')}
      >
        {privacy.devicePermissions.length === 0 ? (
          <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
            {t('settings.privacy.noDevicePermissions', 'No device permissions granted.')}
          </p>
        ) : (
          <div className="space-y-2">
            {privacy.devicePermissions.map((perm) => {
              const accessLabel: Record<DeviceAccessType, string> = {
                direct: 'Direct Access',
                data_routed: 'Data Routed',
              }
              return (
                <div
                  key={perm.id}
                  className="flex items-center justify-between rounded-lg p-3"
                  style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
                >
                  <div className="flex items-center gap-3">
                    <Cpu size={14} style={{ color: 'var(--cp-muted)' }} />
                    <div>
                      <p className="text-sm" style={{ color: 'var(--cp-text)' }}>
                        {perm.appName} · <span className="capitalize">{perm.capability.replace('_', ' ')}</span>
                      </p>
                      <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                        {accessLabel[perm.accessType]}
                      </p>
                    </div>
                  </div>
                  <StatusBadge
                    status={perm.granted ? 'pass' : 'fail'}
                    label={perm.granted ? t('settings.privacy.granted', 'Granted') : t('settings.privacy.denied', 'Denied')}
                  />
                </div>
              )
            })}
          </div>
        )}
      </Section>
    </div>
  )
}
