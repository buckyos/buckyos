import { Copy, Server, Globe, Wifi } from 'lucide-react'
import { Button } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'
import { Section, CollapsibleSection, InfoRow, StatusBadge } from '../components/shared/Section'
import { SettingsPageIntro } from '../components/shared/SettingsPageIntro'

export function ClusterManagerPage() {
  const { t } = useI18n()
  const { cluster } = useSettingsSnapshot()
  const store = useSettingsStore()
  const { overview, nodes, zones, connectivity, certificates } = cluster
  const [copied, setCopied] = useState(false)

  const handleCopyClusterInfo = () => {
    navigator.clipboard.writeText(store.getClusterInfoJSON())
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  return (
    <div className="space-y-4">
      <SettingsPageIntro
        page="cluster"
        title={t('settings.cluster.title', 'Cluster Manager')}
        description={t(
          'settings.cluster.description',
          'View your cluster, network identity, connectivity, and certificate status.',
        )}
      />

      <Section title={t('settings.cluster.overview', 'Cluster Overview')}>
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
          {[
            { label: t('settings.cluster.mode', 'Mode'), value: overview.clusterMode === 'single_node' ? 'Single Node' : 'Multi Node', icon: Server },
            { label: t('settings.cluster.nodes', 'Nodes'), value: overview.nodeCount, icon: Server },
            { label: t('settings.cluster.zones', 'Zones'), value: overview.zoneCount, icon: Globe },
            { label: t('settings.cluster.activeZone', 'Active Zone'), value: overview.activeZone?.split(':').pop() ?? '—', icon: Wifi },
          ].map((item) => (
            <div
              key={item.label}
              className="rounded-xl p-3 text-center"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <item.icon size={16} className="mx-auto mb-1.5" style={{ color: 'var(--cp-muted)' }} />
              <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{item.label}</p>
              <p className="text-sm font-semibold mt-0.5" style={{ color: 'var(--cp-text)' }}>{item.value}</p>
            </div>
          ))}
        </div>
      </Section>

      {/* Nodes / Devices */}
      <Section title={t('settings.cluster.nodesDevices', 'Nodes / Devices')}>
        <div className="space-y-2">
          {nodes.map((node) => (
            <div
              key={node.deviceId}
              className="flex items-center justify-between rounded-lg p-3"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <div>
                <p className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{node.deviceName}</p>
                <p className="text-xs mt-0.5" style={{ color: 'var(--cp-muted)' }}>{node.deviceId}</p>
              </div>
              <StatusBadge
                status={node.status === 'online' ? 'pass' : 'fail'}
                label={node.status === 'online' ? t('settings.cluster.online', 'Online') : t('settings.cluster.offline', 'Offline')}
              />
            </div>
          ))}
        </div>
      </Section>

      {/* Zones */}
      <Section title={t('settings.cluster.zonesTitle', 'Zones')}>
        {zones.map((zone) => (
          <div key={zone.zoneDID} className="space-y-0.5">
            <InfoRow label={t('settings.cluster.zoneDID', 'Zone DID')} value={
              <span className="font-mono text-xs">{zone.zoneDID}</span>
            } />
            <InfoRow label={t('settings.cluster.didMethod', 'DID Method')} value={zone.didMethod} />
            <InfoRow label={t('settings.cluster.ownerDID', 'Owner DID')} value={
              <span className="font-mono text-xs">{zone.ownerDID}</span>
            } />
            <InfoRow label={t('settings.cluster.zoneName', 'Name')} value={zone.name} />
          </div>
        ))}
      </Section>

      {/* Connectivity */}
      <Section title={t('settings.cluster.connectivity', 'Connectivity')}>
        <div className="space-y-0.5">
          <InfoRow
            label={t('settings.cluster.domain', 'Domain')}
            value={connectivity.domain}
          />
          <InfoRow
            label={t('settings.cluster.domainType', 'Domain Type')}
            value={connectivity.domainType === 'bns_subdomain' ? 'BNS Subdomain' : 'Custom Domain'}
          />
          <InfoRow
            label={t('settings.cluster.snRelay', 'SN Relay')}
            value={
              <StatusBadge
                status={connectivity.snRelay ? 'warn' : 'pass'}
                label={connectivity.snRelay
                  ? t('settings.cluster.relayActive', 'Active (via relay)')
                  : t('settings.cluster.directConnection', 'Direct')}
              />
            }
          />
          {connectivity.snRegion && (
            <InfoRow label={t('settings.cluster.snRegion', 'SN Region')} value={connectivity.snRegion} />
          )}
          {connectivity.snTrafficUsed && (
            <InfoRow
              label={t('settings.cluster.snTraffic', 'SN Traffic')}
              value={`${connectivity.snTrafficUsed} / ${connectivity.snTrafficTotal}`}
            />
          )}
          <InfoRow label={t('settings.cluster.ipv4', 'IPv4')} value={
            <StatusBadge status={connectivity.ipv4 ? 'pass' : 'fail'} label={connectivity.ipv4 ? 'Yes' : 'No'} />
          } />
          <InfoRow label={t('settings.cluster.ipv6', 'IPv6')} value={
            <StatusBadge status={connectivity.ipv6 ? 'pass' : 'warn'} label={connectivity.ipv6 ? 'Yes' : 'No'} />
          } />
          <InfoRow label={t('settings.cluster.directConnect', 'Direct Connect')} value={
            <StatusBadge status={connectivity.directConnect ? 'pass' : 'warn'} label={connectivity.directConnect ? 'Yes' : 'No'} />
          } />
          <InfoRow label={t('settings.cluster.portMapping', 'Port Mapping')} value={
            <StatusBadge status={connectivity.portMapping ? 'pass' : 'warn'} label={connectivity.portMapping ? 'Enabled' : 'Disabled'} />
          } />
        </div>

        <CollapsibleSection title={t('settings.cluster.dnsDetails', 'DNS Details')}>
          <p className="font-mono text-xs" style={{ color: 'var(--cp-text)' }}>
            {connectivity.dnsInfo}
          </p>
        </CollapsibleSection>
      </Section>

      {/* Certificates */}
      <Section title={t('settings.cluster.certificates', 'Certificates')}>
        {certificates.map((cert, i) => (
          <div key={i} className="space-y-0.5">
            <InfoRow label={t('settings.cluster.certSource', 'Source')} value={
              <span
                className="px-2 py-0.5 rounded-full text-xs font-medium"
                style={{
                  color: cert.source === 'auto' ? 'var(--cp-success)' : 'var(--cp-accent)',
                  background: cert.source === 'auto'
                    ? 'color-mix(in srgb, var(--cp-success) 14%, transparent)'
                    : 'color-mix(in srgb, var(--cp-accent) 14%, transparent)',
                }}
              >
                {cert.source === 'auto' ? 'Auto (NS managed)' : 'Custom'}
              </span>
            } />
            <InfoRow label={t('settings.cluster.certDomain', 'Domain')} value={cert.domain} />
            <InfoRow label={t('settings.cluster.certIssuer', 'Issuer')} value={cert.issuer} />
            <InfoRow label={t('settings.cluster.certExpiry', 'Expiry')} value={new Date(cert.expiryDate).toLocaleDateString()} />
            <InfoRow label={t('settings.cluster.certStatus', 'Status')} value={
              <StatusBadge status={cert.valid ? 'pass' : 'fail'} label={cert.valid ? 'Valid' : 'Expired'} />
            } />
          </div>
        ))}
        <CollapsibleSection title={t('settings.cluster.x509Details', 'X.509 Raw Certificate')}>
          <pre
            className="text-xs overflow-x-auto p-2 rounded-lg"
            style={{
              color: 'var(--cp-text)',
              background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)',
            }}
          >
            {certificates[0]?.x509Raw ?? 'N/A'}
          </pre>
        </CollapsibleSection>
      </Section>

      {/* Debug & Export */}
      <Section title={t('settings.cluster.debugExport', 'Debug & Export')}>
        <div className="flex flex-wrap gap-2">
          <Button
            size="small"
            variant="outlined"
            startIcon={<Copy size={14} />}
            onClick={handleCopyClusterInfo}
          >
            {copied
              ? t('settings.general.copied', 'Copied!')
              : t('settings.cluster.copyClusterInfo', 'Copy Cluster Info')}
          </Button>
        </div>
      </Section>
    </div>
  )
}
