import { Button, FormControlLabel, Switch, TextField } from '@mui/material'
import { Save } from 'lucide-react'
import { accessDenialOf, DEFAULT_JUMP_BASE_URL } from 'buckyos'
import { useEffect, useState } from 'react'
import { fetchSsoSettings, saveSsoSettings, type SsoSettings } from '../../../api/sso'
import { useI18n } from '../../../i18n/provider'
import { denialText } from '../../../sso/denial'
import { Section, SectionNote } from '../components/shared/Section'
import { useViewerIsAdmin } from '../hooks/use-viewer-is-admin'

interface Draft {
  self_register_enabled: boolean
  did_login_enabled: boolean
  jump_base_url: string
}

function draftOf(settings: SsoSettings): Draft {
  return {
    self_register_enabled: settings.self_register_enabled,
    did_login_enabled: settings.did_login_enabled,
    jump_base_url: settings.jump_base_url?.trim() ?? '',
  }
}

const sameDraft = (left: Draft, right: Draft) =>
  left.self_register_enabled === right.self_register_enabled
  && left.did_login_enabled === right.did_login_enabled
  && left.jump_base_url.trim() === right.jump_base_url.trim()

/** An https origin, or empty for the default jump entry. */
function jumpBaseValid(value: string): boolean {
  const text = value.trim()
  if (!text) return true
  try {
    const url = new URL(text)
    return url.protocol === 'https:' && (url.pathname === '/' || url.pathname === '') && !url.search && !url.hash && !url.username && !url.password
  } catch {
    return false
  }
}

type Status =
  | { kind: 'loading' }
  | { kind: 'load_failed' }
  | { kind: 'ready' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'save_failed'; detail: string }

/** Settings › Sign-in & Sharing (admins): the Zone switches for self-registration and DID login, and the jump entry. */
export function SsoPage() {
  const { t } = useI18n()
  const isAdmin = useViewerIsAdmin()
  const [stored, setStored] = useState<Draft | null>(null)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [status, setStatus] = useState<Status>({ kind: 'loading' })
  const [reloadSeq, setReloadSeq] = useState(0)

  useEffect(() => {
    if (!isAdmin) return
    let live = true
    void fetchSsoSettings().then(({ data }) => {
      if (!live) return
      if (!data) {
        setStatus({ kind: 'load_failed' })
        return
      }
      setStored(draftOf(data))
      setDraft(draftOf(data))
      setStatus({ kind: 'ready' })
    })
    return () => { live = false }
  }, [isAdmin, reloadSeq])

  if (isAdmin === false) {
    return (
      <div className="space-y-4" data-testid="settings-sso-page">
        <Section title={t('sso.settings.title', 'Sign-in & Sharing')}>
          <SectionNote>{t('sso.settings.adminOnly', 'Only Zone administrators can change these settings.')}</SectionNote>
        </Section>
      </div>
    )
  }

  const busy = status.kind === 'loading' || status.kind === 'saving'
  const jumpInvalid = draft ? !jumpBaseValid(draft.jump_base_url) : false
  const dirty = !!draft && !!stored && !sameDraft(draft, stored)

  const update = (patch: Partial<Draft>) => {
    setDraft((current) => (current ? { ...current, ...patch } : current))
    setStatus({ kind: 'ready' })
  }

  const save = async () => {
    if (!draft || jumpInvalid) return
    setStatus({ kind: 'saving' })
    const { data, error } = await saveSsoSettings(draft)
    if (!data) {
      const denial = accessDenialOf(error)
      setStatus({ kind: 'save_failed', detail: denial ? denialText(t, denial) : error instanceof Error ? error.message : String(error ?? '') })
      return
    }
    setStored(draftOf(data))
    setDraft(draftOf(data))
    setStatus({ kind: 'saved' })
  }

  return (
    <div className="space-y-4" data-testid="settings-sso-page">
      <Section
        title={t('sso.settings.methods', 'Ways in for visitors')}
        description={t('sso.settings.methodsHint', 'A share link offers a method only when the link allows it and it is switched on here. Guest access is chosen by each resource owner on the link itself.')}
      >
        {status.kind === 'load_failed' ? (
          <div className="flex flex-wrap items-center gap-2">
            <SectionNote tone="danger">{t('sso.settings.loadFailed', 'The sign-in settings could not be loaded.')}</SectionNote>
            <Button size="small" variant="outlined" onClick={() => { setStatus({ kind: 'loading' }); setReloadSeq((seq) => seq + 1) }}>
              {t('sso.common.retry', 'Retry')}
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <div>
              <FormControlLabel
                control={(
                  <Switch
                    checked={draft?.self_register_enabled ?? true}
                    disabled={busy || !draft}
                    onChange={(event) => update({ self_register_enabled: event.target.checked })}
                    slotProps={{ input: { 'aria-label': t('sso.settings.selfRegister', 'Self-registration') } }}
                    data-testid="settings-sso-self-register"
                  />
                )}
                label={t('sso.settings.selfRegister', 'Self-registration')}
              />
              <SectionNote>{t('sso.settings.selfRegisterHint', 'Visitors may create a restricted account of this Zone from a share link. New accounts join no user group.')}</SectionNote>
            </div>
            <div>
              <FormControlLabel
                control={(
                  <Switch
                    checked={draft?.did_login_enabled ?? true}
                    disabled={busy || !draft}
                    onChange={(event) => update({ did_login_enabled: event.target.checked })}
                    slotProps={{ input: { 'aria-label': t('sso.settings.didLogin', 'DID login') } }}
                    data-testid="settings-sso-did-login"
                  />
                )}
                label={t('sso.settings.didLogin', 'DID login')}
              />
              <SectionNote>{t('sso.settings.didLoginHint', 'People with a DID from another Zone may sign in after their home Zone confirms it; no password is created here.')}</SectionNote>
            </div>
          </div>
        )}
      </Section>

      {status.kind !== 'load_failed' ? (
        <Section
          title={t('sso.settings.jump', 'Jump entry')}
          description={t('sso.settings.jumpHint', 'Where Apps and web pages send people to pick their own Zone. Leave it empty to use the default.')}
        >
          <TextField
            fullWidth
            size="small"
            label={t('sso.settings.jumpLabel', 'Jump base URL')}
            placeholder={DEFAULT_JUMP_BASE_URL}
            value={draft?.jump_base_url ?? ''}
            disabled={busy || !draft}
            error={jumpInvalid}
            helperText={jumpInvalid ? t('sso.settings.jumpInvalid', 'Use an https origin such as https://jump.example.com.') : undefined}
            onChange={(event) => update({ jump_base_url: event.target.value })}
            slotProps={{ htmlInput: { 'data-testid': 'settings-sso-jump' } }}
          />
        </Section>
      ) : null}

      {status.kind !== 'load_failed' ? (
          <div className="flex flex-wrap items-center gap-2">
            <Button size="small" variant="contained" startIcon={<Save size={14} />} disabled={!dirty || busy || jumpInvalid} onClick={() => void save()} data-testid="settings-sso-save">
              {status.kind === 'saving' ? t('settings.zone.saving', 'Saving…') : t('settings.zone.save', 'Save')}
            </Button>
            <Button size="small" variant="text" disabled={!dirty || busy} onClick={() => { setDraft(stored); setStatus({ kind: 'ready' }) }}>
              {t('settings.zone.discard', 'Discard changes')}
            </Button>
            <span className="text-xs" role="status" data-testid="settings-sso-status">
              {status.kind === 'saved' ? (
                <span className="text-[color:var(--cp-success)]">{t('settings.zone.saved', 'Saved.')}</span>
              ) : status.kind === 'save_failed' ? (
                <span className="text-[color:var(--cp-danger)]">{t('settings.zone.saveFailed', 'Not saved: {{detail}}', { detail: status.detail })}</span>
              ) : null}
            </span>
          </div>
      ) : null}
    </div>
  )
}
