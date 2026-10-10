/* ── Step 2: runtime (Loader, template, update policy) ── */

import { useState } from 'react'
import { Alert, Button, Checkbox, CircularProgress, FormControlLabel, IconButton, MenuItem, TextField, Tooltip } from '@mui/material'
import { ChevronDown, ChevronRight, CircleHelp, RefreshCw } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { AgentTemplate } from '../../api/user_mgr'
import type { TemplatesState } from './hooks'
import { AGENT_LOADER_OPENDAN, type AgentSetupDraft } from './model'
import { Section, StatusPill, SummaryRows } from './ui'

function templateSourceKey(template: AgentTemplate) {
  return template.source === 'installed' ? 'agentSetup.template.sourceInstalled' : 'agentSetup.template.sourceBundled'
}

export function InstallTemplateHelp({ onRefresh, refreshing }: { onRefresh: () => void; refreshing: boolean }) {
  const { t } = useI18n()
  const [open, setOpen] = useState(false)
  return (
    <div data-testid="agent-setup-install-template">
      <button
        type="button"
        aria-expanded={open}
        className="inline-flex items-center gap-1.5 text-[13px] font-semibold text-[color:var(--cp-accent)]"
        onClick={() => setOpen((value) => !value)}
      >
        {open ? <ChevronDown size={15} /> : <ChevronRight size={15} />}
        {t('agentSetup.template.install')}
      </button>
      {open ? (
        <div className="mt-2 space-y-2 rounded-[14px] border px-3 py-3 text-[13px] leading-5 text-[color:var(--cp-muted)]" style={{ borderColor: 'var(--cp-border)' }}>
          <p>{t('agentSetup.template.installCli')}</p>
          <pre className="desktop-scrollbar overflow-x-auto rounded-[10px] px-3 py-2 text-[12px] text-[color:var(--cp-text)]" style={{ background: 'color-mix(in srgb, var(--cp-surface-2) 70%, var(--cp-surface))' }}>
            {'buckyos app fetch <pikg> --plan <plan.json>\nbuckyos app install <pikg> --plan <plan.json>'}
          </pre>
          <p>{t('agentSetup.template.installPolicy')}</p>
          <p>{t('agentSetup.template.installReturn')}</p>
          <Button size="small" variant="outlined" disabled={refreshing} startIcon={refreshing ? <CircularProgress size={13} color="inherit" /> : <RefreshCw size={14} />} onClick={onRefresh}>
            {t('agentSetup.template.refresh')}
          </Button>
        </div>
      ) : null}
    </div>
  )
}

export function RuntimePage({
  draft,
  update,
  templates,
  compatibleTemplates,
  selectedTemplate,
  onReloadTemplates,
  summaryRows,
  displayName,
}: {
  draft: AgentSetupDraft
  update: (patch: Partial<AgentSetupDraft>) => void
  templates: TemplatesState
  compatibleTemplates: AgentTemplate[]
  selectedTemplate: AgentTemplate | null
  onReloadTemplates: () => void
  summaryRows: Array<[string, string]>
  displayName: string
}) {
  const { t } = useI18n()
  const [loaderHelp, setLoaderHelp] = useState(false)

  return (
    <div className="space-y-4">
      <Section
        title={t('agentSetup.loader.title')}
        actions={
          <Tooltip title={t('agentSetup.loader.help')}>
            <IconButton size="small" aria-label={t('agentSetup.loader.help')} aria-expanded={loaderHelp} onClick={() => setLoaderHelp((value) => !value)}>
              <CircleHelp size={15} />
            </IconButton>
          </Tooltip>
        }
      >
        <TextField
          select
          label={t('agentSetup.loader.label')}
          value={AGENT_LOADER_OPENDAN}
          inputProps={{ 'data-testid': 'agent-setup-loader' }}
        >
          <MenuItem value={AGENT_LOADER_OPENDAN}>OpenDAN</MenuItem>
        </TextField>
        {loaderHelp ? (
          <div className="mt-3 space-y-1.5 text-[13px] leading-5 text-[color:var(--cp-muted)]" data-testid="agent-setup-loader-help">
            <p>{t('agentSetup.loader.helpOpenDAN')}</p>
            <p>{t('agentSetup.loader.helpExtend')}</p>
            <p className="flex flex-wrap items-center gap-2">
              {t('agentSetup.loader.guide')}
              <StatusPill tone="muted">{t('agentSetup.loader.guideStatus')}</StatusPill>
            </p>
          </div>
        ) : null}
      </Section>

      <Section title={t('agentSetup.template.title')} description={t('agentSetup.template.help')} testId="agent-setup-template-section">
        {templates.status === 'loading' ? (
          <p role="status" className="flex items-center gap-2 text-sm text-[color:var(--cp-muted)]">
            <CircularProgress size={14} color="inherit" />
            {t('agentSetup.template.loading')}
          </p>
        ) : templates.status === 'error' ? (
          <Alert
            severity="error"
            data-testid="agent-setup-template-error"
            action={<Button size="small" variant="text" onClick={onReloadTemplates}>{t('common.retry')}</Button>}
          >
            {t('agentSetup.template.loadFailed', undefined, { detail: templates.error.detail })}
          </Alert>
        ) : compatibleTemplates.length === 0 ? (
          <Alert severity="info" data-testid="agent-setup-template-empty">{t('agentSetup.template.empty')}</Alert>
        ) : (
          <div className="space-y-3">
            <TextField
              select
              label={t('agentSetup.template.label')}
              value={selectedTemplate?.template_id ?? ''}
              inputProps={{ 'data-testid': 'agent-setup-template' }}
              onChange={(event) => update({ templateId: event.target.value })}
            >
              {compatibleTemplates.map((template) => (
                <MenuItem key={template.template_id} value={template.template_id}>
                  {template.show_name || template.name} · {t(templateSourceKey(template))}
                </MenuItem>
              ))}
            </TextField>
            {selectedTemplate ? (
              <div className="rounded-[14px] border px-3 py-3" style={{ borderColor: 'var(--cp-border)' }} data-testid="agent-setup-template-info">
                <div className="mb-2 flex flex-wrap items-center gap-2">
                  <span className="text-sm font-semibold text-[color:var(--cp-text)]">{selectedTemplate.show_name || selectedTemplate.name}</span>
                  <StatusPill tone={selectedTemplate.source === 'installed' ? 'accent' : 'muted'}>{t(templateSourceKey(selectedTemplate))}</StatusPill>
                  <StatusPill tone="muted">v{selectedTemplate.version}</StatusPill>
                </div>
                {selectedTemplate.description ? (
                  <p className="mb-2 text-[13px] leading-5 text-[color:var(--cp-muted)]">{selectedTemplate.description}</p>
                ) : null}
                <SummaryRows rows={[[t('agentSetup.template.appDid'), <code key="did" className="break-all text-[12px]">{selectedTemplate.app_did}</code>]]} />
              </div>
            ) : null}
            <div>
              <FormControlLabel
                control={
                  <Checkbox
                    checked={draft.templateAutoUpdate}
                    onChange={(event) => update({ templateAutoUpdate: event.target.checked })}
                    slotProps={{ input: { 'aria-label': t('agentSetup.template.autoUpdate') } }}
                  />
                }
                label={<span className="text-sm text-[color:var(--cp-text)]">{t('agentSetup.template.autoUpdate')}</span>}
              />
              <p className="pl-8 text-[12px] leading-5 text-[color:var(--cp-muted)]">{t('agentSetup.template.autoUpdateHint')}</p>
              {!draft.templateAutoUpdate ? (
                <Alert severity="warning" className="mt-2" data-testid="agent-setup-autoupdate-warning">{t('agentSetup.template.autoUpdateOff')}</Alert>
              ) : null}
            </div>
          </div>
        )}
        <div className="mt-3">
          <InstallTemplateHelp onRefresh={onReloadTemplates} refreshing={templates.status === 'loading'} />
        </div>
      </Section>

      {selectedTemplate ? (
        <>
          <Section title={t('agentSetup.summary.title')} testId="agent-setup-runtime-summary">
            <SummaryRows rows={summaryRows} />
          </Section>
          <Alert severity="success" data-testid="agent-setup-required-done">
            {t('agentSetup.runtime.requiredDone', undefined, { name: displayName })}
          </Alert>
        </>
      ) : null}
    </div>
  )
}
