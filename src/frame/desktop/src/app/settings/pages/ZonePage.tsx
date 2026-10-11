import { Button, TextField } from '@mui/material'
import { Globe, RotateCcw, Save } from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { fetchCurrentAccount } from '../../../api/account'
import {
  fetchZonePresentation,
  setZonePresentation,
  type ZonePresentationPatch,
  type ZonePresentationResponse,
} from '../../../api/settings'
import { useSudoByPassword } from '../../../components/sudo'
import { useI18n } from '../../../i18n/provider'
import { isMockRuntime } from '../../../runtime'
import { ImagePicker } from '../../../presentation/ImagePicker'
import { ZONE_ICON_IMAGE } from '../../../presentation/imageFile'
import { iconCandidates, presentZone, textValue } from '../../../presentation/model'
import { ObjectAvatar } from '../../../presentation/ObjectIcon'
import { ObjectTitle } from '../../../presentation/ObjectTitle'
import { currentZoneHost, publishZonePresentation } from '../../../presentation/zone'
import { InfoRow, Section } from '../components/shared/Section'
import { SettingsPageIntro } from '../components/shared/SettingsPageIntro'

interface Draft {
  display_name: string
  description: string
  icon: string | null
}

function draftOf(source: ZonePresentationResponse | null): Draft {
  return {
    display_name: source?.settings.display_name ?? '',
    description: source?.settings.description ?? '',
    icon: source?.settings.icon ?? null,
  }
}

/** Only the fields that differ from what is stored; an emptied field is sent as null (back to the default). */
function patchOf(draft: Draft, stored: Draft): ZonePresentationPatch {
  const patch: ZonePresentationPatch = {}
  const name = draft.display_name.trim()
  const description = draft.description.trim()
  if (name !== stored.display_name) patch.display_name = name || null
  if (description !== stored.description) patch.description = description || null
  if (draft.icon !== stored.icon) patch.icon = draft.icon
  return patch
}

type Status =
  | { kind: 'loading' }
  | { kind: 'load_failed' }
  | { kind: 'ready' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'save_failed'; detail: string }

export function ZonePage() {
  const { t } = useI18n()
  const requestSudo = useSudoByPassword()
  const [source, setSource] = useState<ZonePresentationResponse | null>(null)
  const [draft, setDraft] = useState<Draft>(() => draftOf(null))
  const [status, setStatus] = useState<Status>({ kind: 'loading' })
  const [canEdit, setCanEdit] = useState(false)
  const [reloadSeq, setReloadSeq] = useState(0)

  useEffect(() => {
    let cancelled = false
    void Promise.all([fetchZonePresentation(), fetchCurrentAccount().catch(() => null)]).then(([result, account]) => {
      if (cancelled) return
      setCanEdit(account?.user_type === 'admin' || account?.user_type === 'root')
      if (!result.data) {
        setStatus({ kind: 'load_failed' })
        return
      }
      setSource(result.data)
      setDraft(draftOf(result.data))
      setStatus({ kind: 'ready' })
    })
    return () => { cancelled = true }
  }, [reloadSeq])

  const stored = useMemo(() => draftOf(source), [source])
  const defaults = useMemo(() => presentZone(source ? { ...source, settings: {} } : null, { fallbackHost: currentZoneHost() }), [source])
  const preview = presentZone(
    source ? { ...source, settings: { display_name: draft.display_name, description: draft.description, icon: draft.icon } } : null,
    { fallbackHost: currentZoneHost() },
  )
  const patch = patchOf(draft, stored)
  const dirty = Object.keys(patch).length > 0
  const configured = Boolean(stored.display_name || stored.description || stored.icon)
  const busy = status.kind === 'loading' || status.kind === 'saving'

  const save = async (next: ZonePresentationPatch) => {
    if (Object.keys(next).length === 0) return
    setStatus({ kind: 'saving' })
    let sessionToken: string | null = null
    if (!isMockRuntime()) {
      try {
        const grant = await requestSudo({
          aud: 'system-config',
          title: t('settings.zone.sudoTitle', 'Change the Zone name and icon'),
          description: t('settings.zone.sudoDescription', 'Confirm your administrator password to change how this Zone is shown to everyone.'),
          reason: t('settings.zone.sudoReason', 'Update the Zone display settings.'),
          confirmLabel: t('settings.zone.save', 'Save'),
        })
        if (!grant) {
          setStatus({ kind: 'ready' })
          return
        }
        sessionToken = grant.sessionToken
      } catch {
        setStatus({ kind: 'save_failed', detail: t('settings.zone.sudoFailed', 'Unable to request administrator permission.') })
        return
      }
    }
    const { data, error } = await setZonePresentation(next, { sessionToken })
    if (!data) {
      setStatus({ kind: 'save_failed', detail: error instanceof Error ? error.message : String(error ?? '') })
      return
    }
    setSource(data)
    setDraft(draftOf(data))
    publishZonePresentation(data)
    setStatus({ kind: 'saved' })
  }

  return (
    <div className="space-y-4" data-testid="settings-zone-page">
      <SettingsPageIntro
        page="zone"
        title={t('settings.zone.title', 'Zone Name & Icon')}
        description={t('settings.zone.description', 'How this Zone is shown on the sign-in page, in browser tabs and across the Desktop.')}
      />

      <Section
        title={t('settings.zone.display', 'Display')}
        description={canEdit
          ? t('settings.zone.displayHint', 'Leave a field empty to use the default from the Zone document or DID.')
          : t('settings.zone.readOnly', 'Only Zone administrators can change these settings.')}
      >
        <div className="flex items-center gap-3 rounded-[18px] p-3 [background:color-mix(in_srgb,var(--cp-surface)_82%,transparent)]" data-testid="settings-zone-preview">
          <ObjectAvatar title={preview.title} icons={preview.icons} size={48} shape="square" fallbackIcon={<Globe size={22} />} loading={status.kind === 'loading'} decorative />
          <ObjectTitle title={preview.title} description={preview.description} />
        </div>

        {status.kind === 'load_failed' ? (
          <div className="mt-3 flex flex-wrap items-center gap-2 text-sm text-[color:var(--cp-danger)]" role="alert">
            {t('settings.zone.loadFailed', 'The Zone display settings could not be loaded.')}
            <Button size="small" variant="outlined" onClick={() => { setStatus({ kind: 'loading' }); setReloadSeq((seq) => seq + 1) }}>
              {t('settings.zone.retry', 'Retry')}
            </Button>
          </div>
        ) : (
          <div className="mt-4 space-y-4">
            <TextField
              fullWidth
              size="small"
              label={t('settings.zone.name', 'Display name')}
              placeholder={defaults.title}
              value={draft.display_name}
              disabled={!canEdit || busy}
              slotProps={{ htmlInput: { maxLength: 64, 'data-testid': 'settings-zone-name' } }}
              onChange={(event) => { setDraft((current) => ({ ...current, display_name: event.target.value })); setStatus({ kind: 'ready' }) }}
            />
            <TextField
              fullWidth
              size="small"
              multiline
              minRows={2}
              label={t('settings.zone.descriptionField', 'Description (optional)')}
              placeholder={defaults.description}
              value={draft.description}
              disabled={!canEdit || busy}
              slotProps={{ htmlInput: { maxLength: 280, 'data-testid': 'settings-zone-description' } }}
              onChange={(event) => { setDraft((current) => ({ ...current, description: event.target.value })); setStatus({ kind: 'ready' }) }}
            />
            <ImagePicker
              title={preview.title}
              value={draft.icon}
              fallbackIcons={iconCandidates(textValue(source?.document.icon))}
              fallbackIcon={<Globe size={28} />}
              shape="square"
              previewSize={56}
              options={ZONE_ICON_IMAGE}
              disabled={!canEdit || busy}
              labels={{
                choose: t('settings.zone.chooseIcon', 'Choose icon'),
                remove: t('settings.zone.removeIcon', 'Use default icon'),
                usingDefault: t('settings.zone.iconDefault', 'Using the default Zone icon.'),
                usingCustom: t('settings.zone.iconCustom', 'Your image; transparency is kept.'),
              }}
              onChange={(icon) => { setDraft((current) => ({ ...current, icon })); setStatus({ kind: 'ready' }) }}
              testId="settings-zone-icon"
              inputTestId="settings-zone-icon-input"
            />
            {canEdit ? (
              <div className="flex flex-wrap items-center gap-2">
                <Button size="small" variant="contained" startIcon={<Save size={14} />} disabled={!dirty || busy} onClick={() => void save(patch)} data-testid="settings-zone-save">
                  {status.kind === 'saving' ? t('settings.zone.saving', 'Saving…') : t('settings.zone.save', 'Save')}
                </Button>
                <Button size="small" variant="text" disabled={!dirty || busy} onClick={() => { setDraft(stored); setStatus({ kind: 'ready' }) }}>
                  {t('settings.zone.discard', 'Discard changes')}
                </Button>
                <Button
                  size="small"
                  variant="text"
                  startIcon={<RotateCcw size={14} />}
                  disabled={!configured || busy}
                  onClick={() => void save({ display_name: null, description: null, icon: null })}
                  data-testid="settings-zone-reset"
                >
                  {t('settings.zone.reset', 'Restore defaults')}
                </Button>
                <span className="text-xs" role="status" data-testid="settings-zone-status">
                  {status.kind === 'saved' ? (
                    <span className="text-[color:var(--cp-success)]">{t('settings.zone.saved', 'Saved.')}</span>
                  ) : status.kind === 'save_failed' ? (
                    <span className="text-[color:var(--cp-danger)]">{t('settings.zone.saveFailed', 'Not saved: {{detail}}', { detail: status.detail })}</span>
                  ) : null}
                </span>
              </div>
            ) : null}
          </div>
        )}
      </Section>

      <Section
        title={t('settings.zone.identity', 'Identity')}
        description={t('settings.zone.identityHint', 'The name and icon above are display settings only; they never change the Zone DID or domain.')}
      >
        <div className="space-y-0.5">
          <InfoRow label={t('settings.zone.did', 'Zone DID')} value={<span className="break-all font-mono text-xs" data-testid="settings-zone-did">{source?.zone_did ?? '—'}</span>} />
          <InfoRow label={t('settings.zone.domain', 'Domain')} value={<span className="break-all font-mono text-xs">{source?.hostname ?? currentZoneHost()}</span>} />
        </div>
      </Section>
    </div>
  )
}
