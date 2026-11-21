import { Button, FormControlLabel, MenuItem, Switch, TextField } from '@mui/material'
import { useEffect, useState } from 'react'
import { fetchSsoConfig, type OAuthSource } from '../../../api/sso'
import { useI18n } from '../../../i18n/provider'

type Field =
  | 'id'
  | 'display_name'
  | 'client_id'
  | 'secret_ref'
  | 'webhost'
  | 'issuer'
  | 'authorization_endpoint'
  | 'token_endpoint'
  | 'jwks_uri'
  | 'account_endpoint'
  | 'id_field'
  | 'email_field'
  | 'email_verified_field'
const labels: Record<Field, string> = {
  id: 'Source ID',
  display_name: 'Display name',
  client_id: 'Client ID',
  secret_ref: 'Secret file name',
  webhost: 'Identity domain',
  issuer: 'Issuer URL',
  authorization_endpoint: 'Authorization endpoint',
  token_endpoint: 'Token endpoint',
  jwks_uri: 'JWKS URL',
  account_endpoint: 'Account endpoint',
  id_field: 'Stable account ID field',
  email_field: 'Email field',
  email_verified_field: 'Verified-email field',
}
function fields(kind: OAuthSource['kind']): Field[] {
  const result: Field[] = ['display_name', 'client_id', 'secret_ref']
  if (kind === 'google') return result
  return [
    'id',
    ...result,
    'webhost',
    'authorization_endpoint',
    'token_endpoint',
    ...(kind === 'oidc'
      ? ['issuer', 'jwks_uri'] as const
      : ['account_endpoint', 'id_field', 'email_field', 'email_verified_field'] as const),
  ]
}

export function OAuthSourcesEditor(
  { sources, onChange, disabled }: {
    sources: OAuthSource[]
    onChange: (sources: OAuthSource[]) => void
    disabled: boolean
  },
) {
  const { t } = useI18n()
  const [origin, setOrigin] = useState('')
  useEffect(() => {
    let live = true
    void fetchSsoConfig().then(({ data }) => {
      if (live && data?.zone_origin) setOrigin(data.zone_origin)
    })
    return () => {
      live = false
    }
  }, [])
  const update = (i: number, patch: Partial<OAuthSource>) =>
    onChange(sources.map((s, index) => index === i ? { ...s, ...patch } : s))
  return (
    <div className='space-y-4'>
      {sources.map((s, i) => (
        <fieldset disabled={disabled} key={i} className='space-y-3'>
          <TextField
            select
            fullWidth
            size='small'
            label={t('sso.provider.type', 'Provider type')}
            value={s.kind}
            onChange={(e) => {
              const kind = e.target.value as OAuthSource['kind']
              update(i, {
                kind,
                id: kind === 'google' ? 'google' : '',
                display_name: kind === 'google' ? 'Google' : '',
              })
            }}
          >
            <MenuItem value='google'>Google</MenuItem>
            <MenuItem value='oidc'>OpenID Connect</MenuItem>
            <MenuItem value='oauth2'>OAuth 2.0</MenuItem>
          </TextField>
          {fields(s.kind).map((field) => (
            <TextField
              size='small'
              fullWidth
              key={field}
              label={t('sso.provider.' + field, labels[field])}
              value={s[field] ?? ''}
              onChange={(e) => update(i, { [field]: e.target.value })}
            />
          ))}
          {s.kind !== 'google' && (
            <TextField
              size='small'
              fullWidth
              label={t('sso.provider.scopes', 'Requested scopes (space-separated)')}
              defaultValue={s.scopes?.join(' ') ?? ''}
              onChange={(e) => update(i, { scopes: e.target.value.trim().split(/\s+/).filter(Boolean) })}
            />
          )}
          {s.id && origin && (
            <p className='text-sm break-all'>
              {t('sso.provider.callback', 'Register this callback URL')}:{' '}
              {new URL('/sso/oauth/' + encodeURIComponent(s.id) + '/callback', origin).href}
            </p>
          )}
          <FormControlLabel
            control={<Switch checked={s.enabled} onChange={(e) => update(i, { enabled: e.target.checked })} />}
            label={t('sso.provider.enabled', 'Enabled')}
          />
        </fieldset>
      ))}
      <Button
        disabled={disabled}
        onClick={() => {
          const kind = sources.some((s) => s.kind === 'google') ? 'oidc' : 'google'
          onChange([...sources, {
            kind,
            id: kind === 'google' ? 'google' : '',
            display_name: kind === 'google' ? 'Google' : '',
            enabled: true,
            client_id: '',
            secret_ref: '',
          }])
        }}
      >
        {t('sso.provider.add', 'Add login provider')}
      </Button>
    </div>
  )
}
