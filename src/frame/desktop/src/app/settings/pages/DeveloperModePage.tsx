import { Download, Terminal, FolderTree, AlertTriangle, CheckCircle, XCircle, ChevronRight } from 'lucide-react'
import { Button, Switch } from '@mui/material'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { useSudoByPassword } from '../../../components/sudo'
import { isMockRuntime } from '../../../runtime'
import { useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'
import { Section, CollapsibleSection, InfoRow, StatusBadge } from '../components/shared/Section'
import { SettingsPageIntro } from '../components/shared/SettingsPageIntro'
import type { ConfigNode, DiagnosticStatus } from '../mock/types'

const diagnosticIcons: Record<DiagnosticStatus, typeof CheckCircle> = {
  pass: CheckCircle,
  warn: AlertTriangle,
  fail: XCircle,
}

export function DeveloperModePage() {
  const { t } = useI18n()
  const { developer } = useSettingsSnapshot()
  const store = useSettingsStore()
  const requestSudo = useSudoByPassword()
  const [selectedConfig, setSelectedConfig] = useState<ConfigNode | null>(null)
  const [exporting, setExporting] = useState(false)
  const [actionError, setActionError] = useState<string | null>(null)

  const handleExportLogs = () => {
    setExporting(true)
    setTimeout(() => setExporting(false), 2000)
  }

  const handleDeveloperModeChange = async () => {
    const enabled = !developer.modeEnabled
    setActionError(null)
    if (isMockRuntime()) {
      store.toggleDeveloperMode()
      return
    }

    try {
      const grant = await requestSudo({
        aud: 'system-config',
        title: enabled
          ? t('settings.developer.enableDialogTitle', 'Enable Developer Mode')
          : t('settings.developer.disableDialogTitle', 'Disable Developer Mode'),
        description: t(
          'settings.developer.sudoDescription',
          'Confirm your administrator password to change the system developer mode.',
        ),
        reason: enabled
          ? t('settings.developer.enableReason', 'Enable system developer capabilities.')
          : t('settings.developer.disableReason', 'Disable system developer capabilities.'),
        confirmLabel: enabled
          ? t('settings.developer.enableConfirm', 'Enable')
          : t('settings.developer.disableConfirm', 'Disable'),
      })
      if (grant) {
        await store.setDeveloperMode(enabled, grant.sessionToken)
      }
    } catch {
      setActionError(t('settings.developer.sudoFailed', 'Unable to request administrator permission.'))
    }
  }

  return (
    <div className="space-y-4">
      <SettingsPageIntro
        page="developer"
        title={t('settings.developer.title', 'Developer Mode')}
        description={t(
          'settings.developer.description',
          'System diagnostics, configuration, and debug tools.',
        )}
      />

      <Section title={t('settings.developer.modeSwitch', 'Mode')}>
        <div className="flex items-center justify-between">
          <div>
            <p className="text-sm" style={{ color: 'var(--cp-text)' }}>
              {t('settings.developer.developerMode', 'Developer Mode')}
            </p>
            <p className="text-xs mt-0.5" style={{ color: 'var(--cp-muted)' }}>
              {t(
                'settings.developer.requiresSudo',
                'Changing this system setting requires administrator permission.',
              )}
            </p>
          </div>
          <Switch
            checked={developer.modeEnabled}
            disabled={developer.loading || developer.saving}
            inputProps={{ 'aria-label': t('settings.developer.developerMode', 'Developer Mode') }}
            onChange={() => void handleDeveloperModeChange()}
          />
        </div>
        {developer.loading && (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('settings.developer.loading', 'Loading developer mode configuration…')}
          </p>
        )}
        {developer.saving && (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('settings.developer.saving', 'Saving developer mode configuration…')}
          </p>
        )}
        {(developer.loadError || actionError) && (
          <p className="mt-2 text-xs" style={{ color: 'var(--cp-danger)' }}>
            {actionError ?? t('settings.developer.unavailable', 'Developer mode configuration is unavailable.')}
          </p>
        )}
        {(developer.enabledAt || developer.enabledBy) && (
          <div className="mt-2 space-y-0.5">
            {developer.enabledAt && (
              <InfoRow
                label={t('settings.developer.lastEnabledAt', 'Last Enabled')}
                value={new Date(developer.enabledAt).toLocaleString()}
              />
            )}
            {developer.enabledBy && (
              <InfoRow
                label={t('settings.developer.enabledBy', 'Enabled By')}
                value={developer.enabledBy}
              />
            )}
          </div>
        )}
      </Section>

      {developer.modeEnabled && (
        <>
          {/* System Diagnostics */}
          <Section title={t('settings.developer.diagnostics', 'System Diagnostics')}>
            <div className="space-y-1.5">
              {developer.diagnostics.map((item) => {
                const Icon = diagnosticIcons[item.status]
                return (
                  <div key={item.name}>
                    <div
                      className="flex items-center gap-3 rounded-lg p-2.5"
                      style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
                    >
                      <Icon
                        size={16}
                        style={{
                          color: item.status === 'pass' ? 'var(--cp-success)'
                            : item.status === 'warn' ? 'var(--cp-warning)'
                            : 'var(--cp-danger)',
                        }}
                      />
                      <div className="flex-1 min-w-0">
                        <div className="flex items-center justify-between">
                          <span className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                            {item.name}
                          </span>
                          <StatusBadge status={item.status} label={item.status.toUpperCase()} />
                        </div>
                        <p className="text-xs mt-0.5" style={{ color: 'var(--cp-muted)' }}>
                          {item.message}
                        </p>
                      </div>
                    </div>
                    {item.detail && (
                      <CollapsibleSection title={t('settings.developer.detail', 'Detail')}>
                        <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                          {item.detail}
                        </p>
                      </CollapsibleSection>
                    )}
                  </div>
                )
              })}
            </div>
          </Section>

          {/* Config Explorer */}
          <Section title={t('settings.developer.configExplorer', 'Config Explorer')}>
            <div
              className="flex rounded-lg overflow-hidden"
              style={{
                border: '1px solid var(--cp-border)',
                minHeight: 200,
              }}
            >
              {/* Config Tree */}
              <div
                className="w-48 shrink-0 overflow-y-auto p-2"
                style={{ borderRight: '1px solid var(--cp-border)' }}
              >
                {developer.configTree.map((node) => (
                  <ConfigTreeNode
                    key={node.key}
                    node={node}
                    depth={0}
                    selectedKey={selectedConfig?.key ?? null}
                    onSelect={setSelectedConfig}
                  />
                ))}
              </div>
              {/* Content Viewer */}
              <div className="flex-1 p-3 overflow-auto">
                {selectedConfig?.content ? (
                  <pre
                    className="text-xs whitespace-pre-wrap"
                    style={{ color: 'var(--cp-text)' }}
                  >
                    {selectedConfig.content}
                  </pre>
                ) : (
                  <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                    {t('settings.developer.selectConfig', 'Select a configuration file to view its contents.')}
                  </p>
                )}
              </div>
            </div>
          </Section>

          {/* Logs */}
          <Section
            title={t('settings.developer.logs', 'Logs')}
            description={t('settings.developer.logsDesc', 'Download recent system logs for troubleshooting.')}
          >
            <div className="flex items-center gap-3">
              <Button
                size="small"
                variant="outlined"
                startIcon={<Download size={14} />}
                onClick={handleExportLogs}
                disabled={exporting}
              >
                {exporting
                  ? t('settings.developer.exporting', 'Exporting...')
                  : t('settings.developer.downloadLogs', 'Download Logs')}
              </Button>
              {developer.lastLogExport && (
                <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
                  {t('settings.developer.lastExport', 'Last export: {{date}}', {
                    date: new Date(developer.lastLogExport).toLocaleDateString(),
                  })}
                </span>
              )}
            </div>
          </Section>

          {/* CLI Helpers */}
          <Section
            title={t('settings.developer.cliHelpers', 'CLI Helpers')}
            description={t('settings.developer.cliHelpersDesc', 'Common commands for local diagnostics. These are displayed for reference only.')}
          >
            <div className="space-y-1">
              {developer.cliHelpers.map((cmd) => (
                <div
                  key={cmd.command}
                  className="flex items-start gap-3 rounded-lg p-2.5"
                  style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
                >
                  <Terminal size={14} className="mt-0.5 shrink-0" style={{ color: 'var(--cp-accent)' }} />
                  <div>
                    <code className="text-xs font-mono font-semibold" style={{ color: 'var(--cp-text)' }}>
                      {cmd.command}
                    </code>
                    <p className="text-xs mt-0.5" style={{ color: 'var(--cp-muted)' }}>
                      {cmd.description}
                    </p>
                  </div>
                </div>
              ))}
            </div>
          </Section>

          {/* System Tester (placeholder) */}
          <Section
            title={t('settings.developer.systemTester', 'System Tasks / System Tester')}
            description={t('settings.developer.systemTesterDesc', 'API debugging and system capability testing tools.')}
          >
            <div
              className="rounded-lg p-6 text-center"
              style={{ background: 'color-mix(in srgb, var(--cp-surface) 82%, transparent)' }}
            >
              <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
                {t('settings.developer.comingSoon', 'Coming soon. This area will support API debugging and internal system calls.')}
              </p>
            </div>
          </Section>
        </>
      )}
    </div>
  )
}

/* ------------------------------------------------------------------ */
/*  Config Tree Node                                                   */
/* ------------------------------------------------------------------ */

function ConfigTreeNode({
  node,
  depth,
  selectedKey,
  onSelect,
}: {
  node: ConfigNode
  depth: number
  selectedKey: string | null
  onSelect: (node: ConfigNode) => void
}) {
  const [open, setOpen] = useState(depth === 0)
  const isFolder = node.type === 'folder'
  const isSelected = selectedKey === node.key

  return (
    <div>
      <button
        type="button"
        className="flex items-center gap-1.5 w-full text-left py-1 px-1 rounded text-xs transition-colors"
        style={{
          paddingLeft: `${depth * 12 + 4}px`,
          color: isSelected ? 'var(--cp-accent)' : 'var(--cp-text)',
          background: isSelected ? 'color-mix(in srgb, var(--cp-accent) 10%, transparent)' : 'transparent',
        }}
        onClick={() => {
          if (isFolder) {
            setOpen(!open)
          } else {
            onSelect(node)
          }
        }}
      >
        {isFolder ? (
          <ChevronRight
            size={12}
            className="shrink-0 transition-transform"
            style={{ transform: open ? 'rotate(90deg)' : undefined }}
          />
        ) : (
          <span className="w-3" />
        )}
        <FolderTree size={12} className="shrink-0" style={{ color: 'var(--cp-muted)' }} />
        <span className="truncate">{node.label}</span>
      </button>
      {isFolder && open && node.children?.map((child) => (
        <ConfigTreeNode
          key={child.key}
          node={child}
          depth={depth + 1}
          selectedKey={selectedKey}
          onSelect={onSelect}
        />
      ))}
    </div>
  )
}
