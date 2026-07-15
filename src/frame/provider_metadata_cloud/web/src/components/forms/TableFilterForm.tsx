import { zodResolver } from '@hookform/resolvers/zod'
import { useEffect, useRef } from 'react'
import { useForm } from 'react-hook-form'
import { tableFilterSchema, type TableFilterInput } from '../../datamodel/schemas'
import { useI18n } from '../../i18n/provider'

export function TableFilterForm({
  providers,
  apiTypes,
  capabilities,
  onChange,
}: {
  providers: Array<{ key: string; label: string }>
  apiTypes?: string[]
  capabilities?: string[]
  onChange: (value: TableFilterInput) => void
}) {
  const { t } = useI18n()
  const form = useForm<TableFilterInput>({
    resolver: zodResolver(tableFilterSchema),
    defaultValues: {
      search: '',
      providerKey: '',
      apiType: '',
      capability: '',
    },
  })
  const values = form.watch()
  const onChangeRef = useRef(onChange)

  useEffect(() => {
    onChangeRef.current = onChange
  }, [onChange])

  useEffect(() => {
    const parsed = tableFilterSchema.safeParse({
      search: values.search,
      providerKey: values.providerKey,
      apiType: values.apiType,
      capability: values.capability,
    })
    if (parsed.success) {
      onChangeRef.current(parsed.data)
    }
  }, [values.search, values.providerKey, values.apiType, values.capability])

  const inputClass = 'h-10 rounded-md border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] px-3 text-sm text-[color:var(--cp-text)]'

  return (
    <form className="grid gap-3 md:grid-cols-[minmax(180px,1.2fr)_minmax(140px,0.8fr)_minmax(140px,0.8fr)_minmax(140px,0.8fr)]">
      <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
        {t('filter.search', 'Search')}
        <input className={inputClass} {...form.register('search')} />
      </label>
      <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
        {t('filter.provider', 'Provider')}
        <select className={inputClass} {...form.register('providerKey')}>
          <option value="">{t('filter.all', 'All')}</option>
          {providers.map((provider) => (
            <option key={provider.key} value={provider.key}>
              {provider.label}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
        {t('filter.apiType', 'API type')}
        <select className={inputClass} {...form.register('apiType')}>
          <option value="">{t('filter.all', 'All')}</option>
          {(apiTypes ?? []).map((apiType) => (
            <option key={apiType} value={apiType}>
              {apiType}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1 text-xs font-semibold text-[color:var(--cp-muted)]">
        {t('filter.capability', 'Capability')}
        <select className={inputClass} {...form.register('capability')}>
          <option value="">{t('filter.all', 'All')}</option>
          {(capabilities ?? []).map((capability) => (
            <option key={capability} value={capability}>
              {capability}
            </option>
          ))}
        </select>
      </label>
    </form>
  )
}
