/**
 * Preview App settings sheet (PRD §13.8) — rendered inside the app panel so
 * it stays within the window that opened it.
 */

import { FormControlLabel, MenuItem, Radio, RadioGroup, Select, Slider, Switch } from '@mui/material'
import { X } from 'lucide-react'
import type { ReactNode } from 'react'
import { useI18n } from '../../i18n/provider'
import { previewSettingsStore, usePreviewSettings, type PreviewAppSettings } from './settings'

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4 py-2">
      <div className="min-w-0">
        <div className="text-[13px] font-medium text-[color:var(--cp-text)]">{label}</div>
        {hint ? <div className="text-[11px] leading-4 text-[color:var(--cp-muted)]">{hint}</div> : null}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  )
}

export function PreviewSettingsDialog({ onClose }: { onClose: () => void }) {
  const { t } = useI18n()
  const settings = usePreviewSettings()
  const update = (patch: Partial<PreviewAppSettings>) => previewSettingsStore.update(patch)

  return (
    <div className="absolute inset-0 z-40 flex items-center justify-center bg-[color:color-mix(in_srgb,var(--cp-shadow)_30%,transparent)] p-4" onClick={onClose} data-testid="preview-settings">
      <div
        role="dialog"
        aria-label={t('previewApp.settings.title', 'Preview settings')}
        className="flex max-h-full w-full max-w-md flex-col rounded-[20px] border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] shadow-[var(--cp-panel-shadow)]"
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between border-b border-[color:var(--cp-border)] px-4 py-3">
          <div className="text-[14px] font-semibold">{t('previewApp.settings.title', 'Preview settings')}</div>
          <button type="button" onClick={onClose} aria-label={t('common.close', 'Close')} className="rounded-full p-1 hover:bg-[color:var(--cp-surface-2)]">
            <X size={14} />
          </button>
        </div>
        <div className="desktop-scrollbar flex-1 overflow-y-auto px-4 py-2">
          <Row label={t('previewApp.settings.windowMode', 'Automatic window mode')} hint={t('previewApp.settings.windowModeHint', 'How requests from other apps pick a window.')}>
            <RadioGroup row value={settings.windowMode} onChange={(_, value) => update({ windowMode: value as PreviewAppSettings['windowMode'] })}>
              <FormControlLabel value="smart" control={<Radio size="small" />} label={t('previewApp.settings.smart', 'Smart')} />
              <FormControlLabel value="single" control={<Radio size="small" />} label={t('previewApp.settings.single', 'Single window')} />
            </RadioGroup>
          </Row>
          <Row label={t('previewApp.settings.limit', 'Automatic window limit')} hint={t('previewApp.settings.limitHint', 'Manual windows are not counted.')}>
            <div className="flex w-40 items-center gap-3">
              <Slider size="small" min={1} max={16} value={settings.autoWindowLimit} onChange={(_, value) => update({ autoWindowLimit: Array.isArray(value) ? value[0] : value })} data-testid="preview-settings-limit" />
              <span className="w-6 text-right text-[12px] tabular-nums">{settings.autoWindowLimit}</span>
            </div>
          </Row>
          <Row label={t('previewApp.settings.uiMode', 'Default UI mode')}>
            <Select size="small" value={settings.defaultUiMode} onChange={(event) => update({ defaultUiMode: event.target.value as PreviewAppSettings['defaultUiMode'] })}>
              <MenuItem value="auto">{t('previewApp.settings.uiAuto', 'Auto')}</MenuItem>
              <MenuItem value="visible">{t('previewApp.settings.uiVisible', 'Visible')}</MenuItem>
              <MenuItem value="silent">{t('previewApp.settings.uiSilent', 'Silent')}</MenuItem>
            </Select>
          </Row>
          <Row label={t('previewApp.settings.fit', 'Default image layout')}>
            <Select size="small" value={settings.defaultFitMode} onChange={(event) => update({ defaultFitMode: event.target.value as PreviewAppSettings['defaultFitMode'] })}>
              <MenuItem value="contain">{t('previewApp.settings.fitContain', 'Fit to view')}</MenuItem>
              <MenuItem value="actual-size">{t('previewApp.settings.fitActual', 'Actual size')}</MenuItem>
              <MenuItem value="cover">{t('previewApp.settings.fitCover', 'Fill (crop)')}</MenuItem>
            </Select>
          </Row>
          <Row label={t('previewApp.settings.wrapContainer', 'Wrap around in folders')} hint={t('previewApp.settings.wrapHint', 'Previous / Next continue from the other end.')}>
            <Switch checked={settings.containerNavigation === 'wrap'} onChange={(_, checked) => update({ containerNavigation: checked ? 'wrap' : 'bounded' })} />
          </Row>
          <Row label={t('previewApp.settings.wrapList', 'Wrap around in selections')}>
            <Switch checked={settings.listNavigation === 'wrap'} onChange={(_, checked) => update({ listNavigation: checked ? 'wrap' : 'bounded' })} />
          </Row>
          <Row label={t('previewApp.settings.restore', 'Offer to reopen the last session')}>
            <Switch checked={settings.restoreLastSession} onChange={(_, checked) => update({ restoreLastSession: checked })} />
          </Row>
          <Row label={t('previewApp.settings.prefetch', 'Prefetch neighbouring items')}>
            <Switch checked={settings.prefetchAdjacent} onChange={(_, checked) => update({ prefetchAdjacent: checked })} />
          </Row>
          <Row label={t('previewApp.settings.preferFullApp', 'Prefer a dedicated app when installed')}>
            <Switch checked={settings.preferFullApp} onChange={(_, checked) => update({ preferFullApp: checked })} />
          </Row>
        </div>
        <div className="flex justify-end gap-2 border-t border-[color:var(--cp-border)] px-4 py-3">
          <button type="button" onClick={() => previewSettingsStore.reset()} className="rounded-full border border-[color:var(--cp-border)] px-3 py-1.5 text-[12px] hover:border-[color:var(--cp-accent)]">
            {t('common.reset', 'Reset')}
          </button>
          <button type="button" onClick={onClose} className="rounded-full bg-[color:var(--cp-accent)] px-4 py-1.5 text-[12px] font-semibold text-white">
            {t('common.close', 'Close')}
          </button>
        </div>
      </div>
    </div>
  )
}
