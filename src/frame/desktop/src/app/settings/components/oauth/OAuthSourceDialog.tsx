import { Alert, Button, Dialog, DialogActions, DialogContent, DialogTitle, Radio, TextField } from '@mui/material'
import { useState, type ReactNode } from 'react'
import type { OAuthSource } from '../../../../api/sso'
import { useI18n } from '../../../../i18n/provider'
import { CopyButton } from '../shared/CopyButton'
import {
  callbackUrl,
  FIELD_LABELS,
  fieldError,
  formFields,
  KIND_LABELS,
  lockedFields,
  newSource,
  sourceComplete,
  type Field,
  type SourceKind,
} from './model'

const KINDS: SourceKind[] = ['google', 'oidc', 'oauth2']

type Translate = ReturnType<typeof useI18n>['t']

/** Where the service reads a source's client secret, with a copy of the path. */
function SecretLocation({ secretRef, t }: { secretRef: string; t: Translate }) {
  const path = `$BUCKYOS_ROOT/security/oauth/${secretRef || t('sso.provider.secretPlaceholder', '<secret file name>')}`
  return (
    <code dir="ltr" className="my-1 block break-all select-all text-sm" style={{ color: 'var(--cp-text)' }}>
      {path}
    </code>
  )
}

function CallbackLine({ origin, id, t }: { origin: string; id: string; t: Translate }) {
  if (!origin) return null
  if (!id) {
    return (
      <code dir="ltr" className="my-1 block break-all text-sm" style={{ color: 'var(--cp-text)' }}>
        {callbackUrl(origin, 'ID').replace('/ID/', `/${t('sso.provider.idPlaceholder', '<source ID>')}/`)}
      </code>
    )
  }
  const url = callbackUrl(origin, id)
  return (
    <div className="my-1 flex flex-wrap items-center gap-2">
      <code dir="ltr" className="min-w-0 break-all select-all text-sm" style={{ color: 'var(--cp-text)' }} data-testid="sso-provider-callback">
        {url}
      </code>
      <CopyButton label={t('settings.common.copy', 'Copy')} text={() => url} />
    </div>
  )
}

function Checklist({ items }: { items: ReactNode[] }) {
  return (
    <ol className="list-decimal space-y-2 pl-5 text-sm leading-6" style={{ color: 'var(--cp-text)' }}>
      {items.map((item, index) => <li key={index}>{item}</li>)}
    </ol>
  )
}

/** Step one of adding: what to set up at the provider before the form. */
function Preparation({ kind, origin, t }: { kind: SourceKind; origin: string; t: Translate }) {
  const muted = (text: string) => <span style={{ color: 'var(--cp-muted)' }}>{text}</span>
  if (kind === 'google') {
    return (
      <Checklist items={[
        t('sso.provider.prep.google.project', 'A Google Cloud project with the OAuth consent screen set up.'),
        <>
          {t('sso.provider.prep.google.client', 'Under Credentials, an OAuth client of type “Web application”, with this address in Authorized redirect URIs:')}
          <CallbackLine origin={origin} id="google" t={t} />
        </>,
        t('sso.provider.prep.google.clientId', 'The client ID. You enter it in the next step.'),
        <>
          {t('sso.provider.googleSecretLocation', 'On the server running Control Panel, save the Google client secret in this file and give Control Panel read access.')}
          <SecretLocation secretRef="google-demo" t={t} />
          {muted(t('sso.provider.googleSecretContents', 'On Linux, $BUCKYOS_ROOT is usually /opt/buckyos. If you downloaded Google’s credentials JSON, copy only the web.client_secret value, without quotation marks.'))}
        </>,
      ]} />
    )
  }
  const common = [
    <>
      {t('sso.provider.prep.custom.client', 'A web application (OAuth client) registered at the provider, with this redirect address. You choose the source ID in the next step.')}
      <CallbackLine origin={origin} id="" t={t} />
    </>,
    <>
      {t('sso.provider.prep.custom.secret', 'Its client ID and client secret. Save the secret on the server running Control Panel, in a file Control Panel can read:')}
      <SecretLocation secretRef="" t={t} />
    </>,
  ]
  if (kind === 'oidc') {
    return (
      <Checklist items={[
        ...common,
        t('sso.provider.prep.oidc.endpoints', 'The issuer URL, authorization endpoint, token endpoint and JWKS URL. They are listed at <issuer>/.well-known/openid-configuration.'),
        t('sso.provider.prep.oidc.domain', 'An identity domain for these accounts’ DIDs. It must contain the issuer’s host, for example example.com for https://login.example.com.'),
        t('sso.provider.prep.oidc.email', 'The provider must return a verified email (the email and email_verified claims).'),
      ]} />
    )
  }
  return (
    <Checklist items={[
      ...common,
      t('sso.provider.prep.oauth2.endpoints', 'The authorization endpoint, token endpoint, and an account endpoint that returns the signed-in user’s profile as JSON.'),
      t('sso.provider.prep.oauth2.fields', 'Three field names in that profile: a stable account ID, the email, and whether the email is verified.'),
      t('sso.provider.prep.oauth2.domain', 'An identity domain for these accounts’ DIDs. It must contain the account endpoint’s host.'),
    ]} />
  )
}

function SourceForm({
  source,
  onChange,
  editing,
  origin,
  touched,
  t,
}: {
  source: OAuthSource
  onChange: (patch: Partial<OAuthSource>) => void
  editing: boolean
  origin: string
  touched: boolean
  t: Translate
}) {
  const locked = editing ? lockedFields(source.kind) : new Set<Field>()
  const [scopes, setScopes] = useState(source.scopes?.join(' ') ?? '')
  return (
    <div className="space-y-4">
      {formFields(source.kind).map((field, index) => {
        const value = String(source[field] ?? '')
        const error = touched || value ? fieldError(source, field, t) : null
        const helper = locked.has(field)
          ? t('sso.provider.fixed', 'Cannot be changed after the source is added.')
          : field === 'id'
            ? t('sso.provider.idHint', 'Lowercase letters, digits, - and _. It is part of the redirect address and cannot be changed later.')
            : field === 'webhost'
              ? t('sso.provider.webhostHint', 'The domain of these accounts’ DIDs. It must contain the issuer’s host and cannot be changed later.')
              : field === 'id_field' && editing
                ? t('sso.provider.idFieldHint', 'Accounts already bound to this source stop signing in if this field changes.')
                : null
        return (
          <TextField
            key={field}
            fullWidth
            size="small"
            autoFocus={index === 0 && !locked.has(field)}
            label={t('sso.provider.' + field, FIELD_LABELS[field])}
            value={value}
            disabled={locked.has(field)}
            error={!!error}
            helperText={error ?? helper ?? undefined}
            onChange={(event) => onChange({ [field]: field === 'webhost' ? event.target.value.trim().toLowerCase() : event.target.value })}
            slotProps={{ htmlInput: { 'data-testid': `sso-provider-field-${field}`, dir: field === 'display_name' ? undefined : 'ltr' } }}
          />
        )
      })}
      {source.kind !== 'google' && (
        <TextField
          fullWidth
          size="small"
          label={t('sso.provider.scopes', 'Requested scopes (space-separated)')}
          value={scopes}
          helperText={source.kind === 'oidc' ? t('sso.provider.scopesHint', 'Optional. openid and email are always requested.') : t('sso.provider.scopesOptional', 'Optional.')}
          onChange={(event) => {
            setScopes(event.target.value)
            onChange({ scopes: event.target.value.trim().split(/\s+/).filter(Boolean) })
          }}
          slotProps={{ htmlInput: { dir: 'ltr', 'data-testid': 'sso-provider-field-scopes' } }}
        />
      )}
      <div className="text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
        <p>{t('sso.provider.callback', 'Register this callback URL')}</p>
        <CallbackLine origin={origin} id={fieldError(source, 'id', t) ? '' : source.id} t={t} />
      </div>
      <div className="text-sm leading-6" style={{ color: 'var(--cp-muted)' }}>
        <p>
          {source.kind === 'google'
            ? t('sso.provider.googleSecretLocation', 'On the server running Control Panel, save the Google client secret in this file and give Control Panel read access.')
            : t('sso.provider.secretLocation', 'On the server running Control Panel, save the client secret in this file and give Control Panel read access.')}
        </p>
        <SecretLocation secretRef={source.secret_ref} t={t} />
        {source.kind === 'google' && <p>{t('sso.provider.googleSecretContents')}</p>}
      </div>
    </div>
  )
}

/**
 * Adding (choose a type, see what to prepare, then fill in everything) or editing a login source.
 * `onSave` stores the source and resolves to an error text, or null once saved. Give it a new `key`
 * for each opening: its state starts from the props.
 */
export function OAuthSourceDialog({
  open,
  editing,
  googleTaken,
  origin,
  onClose,
  onSave,
}: {
  open: boolean
  /** The stored source being edited; null to add one. */
  editing: OAuthSource | null
  googleTaken: boolean
  origin: string
  onClose: () => void
  onSave: (source: OAuthSource) => Promise<string | null>
}) {
  const { t } = useI18n()
  const first: SourceKind = editing?.kind ?? (googleTaken ? 'oidc' : 'google')
  const [step, setStep] = useState<'kind' | 'form'>(editing ? 'form' : 'kind')
  const [kind, setKind] = useState<SourceKind>(first)
  const [source, setSource] = useState<OAuthSource>(() => (editing ? { ...editing } : newSource(first)))
  const [touched, setTouched] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const toForm = () => {
    // Keep what was typed when coming back to the same type.
    if (source.kind !== kind) setSource(newSource(kind))
    setTouched(false)
    setError(null)
    setStep('form')
  }

  const save = async () => {
    setTouched(true)
    if (!sourceComplete(source, t)) return
    setBusy(true)
    setError(null)
    const trimmed = Object.fromEntries(
      Object.entries(source).map(([key, value]) => [key, typeof value === 'string' ? value.trim() : value]),
    ) as unknown as OAuthSource
    const failure = await onSave(trimmed)
    setBusy(false)
    if (failure) setError(failure)
  }

  const title = editing
    ? t('sso.provider.editTitle', 'Edit {{name}}', { name: editing.display_name })
    : step === 'kind'
      ? t('sso.provider.add', 'Add login provider')
      : t('sso.provider.addKindTitle', 'Add {{kind}} sign-in', { kind: KIND_LABELS[kind] })

  return (
    <Dialog open={open} onClose={busy ? undefined : onClose} fullWidth maxWidth="sm" data-testid="sso-provider-dialog">
      <DialogTitle>
        {title}
        {!editing && (
          <span className="block text-sm font-normal" style={{ color: 'var(--cp-muted)' }}>
            {step === 'kind'
              ? t('sso.provider.step1', 'Step 1 of 2: choose a type')
              : t('sso.provider.step2', 'Step 2 of 2: fill in the details')}
          </span>
        )}
      </DialogTitle>
      <DialogContent className="space-y-4">
        {step === 'kind' ? (
          <>
            <div role="radiogroup" aria-label={t('sso.provider.type', 'Provider type')} className="space-y-2 pt-1">
              {KINDS.map((option) => {
                const taken = option === 'google' && googleTaken
                const selected = option === kind
                return (
                  <label
                    key={option}
                    className={`flex items-start gap-2 rounded-lg border p-2 pr-3 ${taken ? 'cursor-default' : 'cursor-pointer'}`}
                    style={{
                      borderColor: selected ? 'var(--cp-accent)' : 'var(--cp-border)',
                      background: selected ? 'color-mix(in srgb, var(--cp-accent) 8%, transparent)' : undefined,
                      opacity: taken ? 0.6 : 1,
                    }}
                    data-testid={`sso-provider-kind-${option}`}
                  >
                    <Radio
                      size="small"
                      checked={selected}
                      disabled={taken}
                      onChange={() => setKind(option)}
                      value={option}
                      slotProps={{ input: { 'aria-label': KIND_LABELS[option] } }}
                    />
                    <span className="pt-1.5">
                      <span className="block font-medium" style={{ color: 'var(--cp-text)' }}>{KIND_LABELS[option]}</span>
                      <span className="block text-sm leading-5" style={{ color: 'var(--cp-muted)' }}>
                        {taken
                          ? t('sso.provider.googleTaken', 'Already added. A Zone has one Google source.')
                          : t('sso.provider.kindHint.' + option)}
                      </span>
                    </span>
                  </label>
                )
              })}
            </div>
            <div>
              <h4 className="mb-2 text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
                {t('sso.provider.prepare', 'Before you continue, have these ready')}
              </h4>
              <Preparation kind={kind} origin={origin} t={t} />
            </div>
          </>
        ) : (
          <>
            {error ? <Alert severity="error" onClose={() => setError(null)}>{error}</Alert> : null}
            <div className="pt-2">
              <SourceForm
                key={source.kind}
                source={source}
                editing={!!editing}
                origin={origin}
                touched={touched}
                t={t}
                onChange={(patch) => setSource((current) => ({ ...current, ...patch }))}
              />
            </div>
          </>
        )}
      </DialogContent>
      <DialogActions>
        {step === 'kind' ? (
          <>
            <Button onClick={onClose}>{t('settings.common.cancel', 'Cancel')}</Button>
            <Button variant="contained" onClick={toForm} data-testid="sso-provider-ready">
              {t('sso.provider.ready', 'I have these ready')}
            </Button>
          </>
        ) : (
          <>
            {editing
              ? <Button disabled={busy} onClick={onClose}>{t('settings.common.cancel', 'Cancel')}</Button>
              : <Button disabled={busy} onClick={() => { setError(null); setStep('kind') }}>{t('sso.provider.back', 'Back')}</Button>}
            <Button variant="contained" disabled={busy} onClick={() => void save()} data-testid="sso-provider-save">
              {busy
                ? t('settings.zone.saving', 'Saving…')
                : editing
                  ? t('settings.zone.save', 'Save')
                  : t('sso.provider.finish', 'Done')}
            </Button>
          </>
        )}
      </DialogActions>
    </Dialog>
  )
}
