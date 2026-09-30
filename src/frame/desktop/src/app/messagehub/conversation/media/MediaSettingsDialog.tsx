import { FormControlLabel, Radio, RadioGroup, Switch } from '@mui/material'
import { X } from 'lucide-react'
import type { ReactNode } from 'react'
import { useI18n } from '../../../../i18n/provider'
import { DialogFocus } from '../../SessionDialogs'
import { mediaSettingsStore, useMediaSettings, type MediaPreviewTarget } from './settings'

function Row({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1 py-2">
      <div className="text-[13px] font-medium text-[color:var(--cp-text)]">{label}</div>
      {hint ? <div className="text-[11px] leading-4 text-[color:var(--cp-muted)]">{hint}</div> : null}
      <div>{children}</div>
    </div>
  )
}

export function MediaSettingsDialog({ canUseWindow, onClose }: { canUseWindow: boolean; onClose: () => void }) {
  const { t } = useI18n()
  const settings = useMediaSettings()
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center p-4" style={{ background: 'color-mix(in srgb, var(--cp-shadow) 30%, transparent)' }} onClick={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={t('messagehub.media.settings', 'Media settings')}
        data-testid="messagehub-media-settings"
        className="w-full max-w-sm rounded-[20px] border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] shadow-[var(--cp-panel-shadow)]"
        onClick={event => event.stopPropagation()}
      >
        <DialogFocus onCancel={onClose}>
          <div className="flex items-center justify-between border-b border-[color:var(--cp-border)] px-4 py-3">
            <div className="text-[14px] font-semibold">{t('messagehub.media.settings', 'Media settings')}</div>
            <button type="button" onClick={onClose} aria-label={t('common.close', 'Close')} className="rounded-full p-1 hover:bg-[color:var(--cp-surface-2)]">
              <X size={14} />
            </button>
          </div>
          <div className="px-4 py-2">
            <Row label={t('messagehub.media.openTarget', 'Open attachments in')} hint={canUseWindow ? undefined : t('messagehub.media.windowUnavailable', 'The Preview window is only available on the desktop.')}>
              <RadioGroup value={settings.previewTarget} onChange={(_, value) => mediaSettingsStore.update({ previewTarget: value as MediaPreviewTarget })}>
                <FormControlLabel value="overlay" control={<Radio size="small" />} label={t('messagehub.media.targetOverlay', 'Pop-up in this window')} />
                <FormControlLabel value="window" disabled={!canUseWindow} control={<Radio size="small" />} label={t('messagehub.media.targetWindow', 'Preview window')} />
              </RadioGroup>
            </Row>
            <Row label={t('messagehub.media.autoplayGif', 'Autoplay GIFs')} hint={t('messagehub.media.autoplayGifHint', 'When off, GIFs show their first frame until opened.')}>
              <Switch
                checked={settings.autoplayGif}
                onChange={(_, checked) => mediaSettingsStore.update({ autoplayGif: checked })}
                slotProps={{ input: { 'aria-label': t('messagehub.media.autoplayGif', 'Autoplay GIFs') } }}
              />
            </Row>
          </div>
          <div className="flex justify-end border-t border-[color:var(--cp-border)] px-4 py-3">
            <button type="button" onClick={onClose} data-autofocus className="rounded-full bg-[color:var(--cp-accent)] px-4 py-1.5 text-[12px] font-semibold text-white">
              {t('common.close', 'Close')}
            </button>
          </div>
        </DialogFocus>
      </div>
    </div>
  )
}
