import { TextField } from '@mui/material'
import { ExternalLink } from 'lucide-react'
import type { DnsProvider } from '../../../../api/zone_domain'
import { useI18n } from '../../../../i18n/provider'
import { SectionNote } from '../shared/Section'
import { Muted } from './parts'

/** The DNS service and the token fields its table asks for, in the service's own words. */
export function ProviderForm({ providers, provider, onProvider, fields, onFields, disabled }: {
  providers: DnsProvider[]
  provider: DnsProvider | null
  onProvider: (id: string) => void
  fields: Record<string, string>
  onFields: (fields: Record<string, string>) => void
  disabled: boolean
}) {
  const { t } = useI18n()
  return (
    <div className="space-y-3" data-testid="domain-provider-form">
      <TextField
        select
        fullWidth
        size="small"
        label={t('settings.domain.provider')}
        value={provider?.id ?? ''}
        disabled={disabled}
        onChange={(event) => onProvider(event.target.value)}
        slotProps={{ select: { native: true }, inputLabel: { shrink: true }, htmlInput: { 'data-testid': 'domain-provider' } }}
      >
        <option value="">{t('settings.domain.providerChoose')}</option>
        {providers.map((item) => (
          <option key={item.id} value={item.id}>
            {item.tested ? item.name : t('settings.domain.providerUntested', undefined, { name: item.name })}
          </option>
        ))}
      </TextField>
      {provider ? (
        <>
          {provider.tested ? null : <SectionNote tone="warning">{t('settings.domain.untestedHint')}</SectionNote>}
          <a
            href={provider.token_url}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 text-sm text-[color:var(--cp-accent)] underline-offset-2 hover:underline"
            data-testid="domain-token-link"
          >
            {t('settings.domain.tokenCreate', undefined, { provider: provider.name })}
            <ExternalLink size={14} />
          </a>
          {provider.fields.map((field) => {
            const common = {
              fullWidth: true,
              size: 'small' as const,
              label: field.optional ? t('settings.domain.optional', undefined, { label: field.label }) : field.label,
              value: fields[field.id] ?? '',
              disabled,
              onChange: (event: { target: { value: string } }) => onFields({ ...fields, [field.id]: event.target.value }),
            }
            const testId = { 'data-testid': `domain-field-${field.id}` }
            return field.options.length ? (
              <TextField key={field.id} {...common} select slotProps={{ select: { native: true }, inputLabel: { shrink: true }, htmlInput: testId }}>
                <option value="" />
                {field.options.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
              </TextField>
            ) : (
              <TextField
                key={field.id}
                {...common}
                multiline={field.multiline}
                minRows={field.multiline ? 4 : undefined}
                maxRows={field.multiline ? 10 : undefined}
                type={field.secret && !field.multiline ? 'password' : 'text'}
                autoComplete={field.secret ? 'new-password' : 'off'}
                slotProps={{ htmlInput: { ...testId, dir: 'ltr', spellCheck: false, autoCapitalize: 'off', autoCorrect: 'off' } }}
              />
            )
          })}
          {provider.token_scope === 'account' ? <SectionNote tone="warning">{t('settings.domain.tokenScopeAccount')}</SectionNote> : null}
          <Muted>{t('settings.domain.tokenStored')}</Muted>
        </>
      ) : null}
    </div>
  )
}
