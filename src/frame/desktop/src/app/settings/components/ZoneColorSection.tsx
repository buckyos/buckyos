/* ── Settings › Appearance › Zone color: the user's colour for this Zone, with where it is used ── */

import { Button, TextField } from '@mui/material'
import clsx from 'clsx'
import { Check, RotateCcw } from 'lucide-react'
import { useState } from 'react'
import { useI18n } from '../../../i18n/provider'
import { ObjectAvatar } from '../../../presentation/ObjectIcon'
import { useZonePresentation } from '../../../presentation/zone'
import { accentPresets, accentText, normalizeAccent } from '../../../shared/appearance/accent.ts'
import { useSettingsSnapshot, useSettingsStore } from '../hooks/use-settings-store'
import { Section } from './shared/Section'

/** What the colour picker starts from when no colour is chosen (the light theme's accent). */
const DEFAULT_SWATCH = accentPresets[0]

export function ZoneColorSection() {
  const { t } = useI18n()
  const { accent } = useSettingsSnapshot()
  const store = useSettingsStore()
  /** The code being typed while it is not a colour yet. */
  const [draft, setDraft] = useState<string | null>(null)
  const choose = (color: string | null) => {
    setDraft(null)
    store.setAccent(color)
  }
  const type = (text: string) => {
    const color = normalizeAccent(text.trim().startsWith('#') ? text.trim() : `#${text.trim()}`)
    if (color) choose(color)
    else setDraft(text)
  }

  return (
    <Section
      title={t('settings.accent.title', 'Zone color')}
      description={t('settings.accent.description', 'Marks this Zone in the BuckyOS APP and is the accent color apps receive. The light and dark themes stay the same. It is saved with your account; other users do not see it.')}
      testId="settings-accent"
      actions={accent ? (
        <Button size="small" variant="text" startIcon={<RotateCcw size={14} />} onClick={() => choose(null)} data-testid="settings-accent-reset">
          {t('settings.accent.default', 'Default')}
        </Button>
      ) : undefined}
    >
      <div className="flex flex-wrap items-center gap-2" role="group" aria-label={t('settings.accent.presets', 'Colors')}>
        {accentPresets.map((color) => {
          const selected = accent === color
          return (
            <button
              key={color}
              type="button"
              aria-pressed={selected}
              aria-label={color}
              title={color}
              data-testid={`settings-accent-${color.slice(1)}`}
              onClick={() => choose(color)}
              className={clsx('inline-flex size-9 items-center justify-center rounded-full border', selected ? 'border-[color:var(--cp-text)]' : 'border-[color:var(--cp-border)]')}
              style={{ background: color, color: accentText(color) }}
            >
              {selected ? <Check size={16} aria-hidden="true" /> : null}
            </button>
          )
        })}
      </div>

      <div className="mt-4 flex flex-wrap items-start gap-3">
        <input
          type="color"
          aria-label={t('settings.accent.custom', 'Custom color')}
          title={t('settings.accent.custom', 'Custom color')}
          value={accent ?? DEFAULT_SWATCH}
          onChange={(event) => choose(normalizeAccent(event.target.value))}
          className="h-10 w-14 cursor-pointer rounded-lg border border-[color:var(--cp-border)] bg-transparent p-1"
          data-testid="settings-accent-picker"
        />
        <TextField
          size="small"
          className="!w-40"
          label={t('settings.accent.code', 'Color code')}
          placeholder="#3366cc"
          value={draft ?? accent ?? ''}
          error={draft !== null && draft.trim() !== ''}
          helperText={draft !== null && draft.trim() !== '' ? t('settings.accent.invalid', 'Enter a color such as #3366cc.') : undefined}
          onChange={(event) => type(event.target.value)}
          onBlur={() => setDraft(null)}
          inputProps={{ 'data-testid': 'settings-accent-code', dir: 'ltr', spellCheck: false }}
        />
      </div>

      <AccentPreview accent={accent} />
    </Section>
  )
}

/** Where the colour shows: the Zone's mark in a list, on an app window, and an app using it as its accent. */
function AccentPreview({ accent }: { accent: string | null }) {
  const { t } = useI18n()
  const zone = useZonePresentation()
  const color = accent ?? 'var(--cp-accent)'
  const onColor = accent ? accentText(accent) : 'var(--cp-surface-opaque)'
  const captionClass = 'mb-1.5 text-xs text-[color:var(--cp-muted)]'
  const boxClass = 'flex h-11 items-center gap-2 overflow-hidden rounded-lg border border-[color:var(--cp-border)] bg-[color:var(--cp-surface-opaque)] px-3 text-sm'
  return (
    <div className="mt-4 grid gap-3 sm:grid-cols-3" aria-label={t('settings.accent.preview', 'Preview')} role="group" data-testid="settings-accent-preview" data-accent={accent ?? ''}>
      <div>
        <p className={captionClass}>{t('settings.accent.previewList', 'In the Zone list')}</p>
        <div className={boxClass}>
          <ObjectAvatar title={zone.title} icons={zone.icons} size={22} shape="square" tone={color} decorative />
          <span className="min-w-0 flex-1 truncate">{zone.title}</span>
          <span className="size-2.5 shrink-0 rounded-full" style={{ background: color }} data-testid="settings-accent-dot" />
        </div>
      </div>
      <div>
        <p className={captionClass}>{t('settings.accent.previewWindow', 'On app windows')}</p>
        <div className={boxClass} style={{ boxShadow: `inset 0 2px 0 ${color}` }}>
          <span className="min-w-0 flex-1 truncate">{t('apps.files', 'Files')}</span>
          <span className="max-w-[60%] shrink-0 truncate rounded-full px-2 py-0.5 text-xs" style={{ background: color, color: onColor }}>{zone.title}</span>
        </div>
      </div>
      <div>
        <p className={captionClass}>{t('settings.accent.previewApps', 'In apps')}</p>
        <div className={boxClass}>
          <span className="rounded-md px-3 py-1 text-xs font-medium" style={{ background: color, color: onColor }}>{t('settings.accent.previewButton', 'Button')}</span>
          <span className="h-1.5 min-w-0 flex-1 rounded-full bg-[color:color-mix(in_srgb,var(--cp-text)_10%,transparent)]">
            <span className="block h-full w-3/5 rounded-full" style={{ background: color }} />
          </span>
        </div>
      </div>
    </div>
  )
}
