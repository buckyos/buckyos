import { useState } from 'react'
import { useI18n } from '../../../../i18n/provider'
import type { PolicyApi } from '../../../../api/aicc_policy'
import {
  PRICE_UNITS,
  priceForm,
  pricingOfForm,
  type PriceEntry,
  type PriceForm,
  type PriceGetResponse,
  type PriceUnit,
} from '../../datamodel/policy'
import { Modal, Segmented } from '../policy/PolicyWidgets'
import { buttonStyle, errorText, fieldStyle, usePriceUnitLabel } from '../policy/policy-labels'

interface PriceEntryDialogProps {
  api: PolicyApi
  entryKey: string
  /** What the key means for people (`openai · gpt-5.1`, an exact model). */
  label: string
  revision: number
  entry?: PriceEntry
  onClose: () => void
  onSaved: (prices: PriceGetResponse) => void
}

type Mode = 'tokens' | 'unit' | 'json'

function RateInput({ label, value, onChange, testId }: { label: string; value: string; onChange: (value: string) => void; testId: string }) {
  const invalid = value.trim() !== '' && !/^\d+(\.\d+)?$/.test(value.trim())
  return (
    <label className="flex min-w-0 flex-1 flex-col gap-1 text-sm">
      <span style={{ color: 'var(--cp-text)' }}>{label}</span>
      <input data-testid={testId} value={value} inputMode="decimal" onChange={(event) => onChange(event.target.value)} className="min-h-10 rounded-lg px-3" style={{ ...fieldStyle, borderColor: invalid ? 'var(--cp-danger)' : 'var(--cp-border)' }} />
    </label>
  )
}

/** One price list entry: Credit per million tokens, Credit per unit, or the full price as JSON (tiers, rules). */
export function PriceEntryDialog({ api, entryKey, label, revision, entry, onClose, onSaved }: PriceEntryDialogProps) {
  const { t } = useI18n()
  const unitLabel = usePriceUnitLabel()
  const initialForm = entry ? priceForm(entry.pricing) : { mode: 'tokens' as const, input: '', output: '', cacheInput: '' }
  const [mode, setMode] = useState<Mode>(initialForm ? initialForm.mode : 'json')
  const [tokens, setTokens] = useState(initialForm?.mode === 'tokens' ? initialForm : { mode: 'tokens' as const, input: '', output: '', cacheInput: '' })
  const [unit, setUnit] = useState<Extract<PriceForm, { mode: 'unit' }>>(initialForm?.mode === 'unit' ? initialForm : { mode: 'unit', unit: 'image', amount: '' })
  const [json, setJson] = useState(() => JSON.stringify(entry?.pricing ?? { currency: 'CREDIT', input_token: 0, output_token: 0 }, null, 2))
  const [note, setNote] = useState(entry?.note ?? '')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  let pricing: unknown = null
  let jsonError = false
  if (mode === 'tokens') pricing = pricingOfForm(tokens)
  else if (mode === 'unit') pricing = pricingOfForm(unit)
  else {
    try { pricing = JSON.parse(json) } catch { jsonError = true }
  }

  const run = async (work: () => Promise<PriceGetResponse>) => {
    setBusy(true)
    setError(null)
    try {
      onSaved(await work())
    } catch (reason) {
      const code = (reason as { code?: string }).code
      setError(code === 'settings_revision_conflict' ? t('aiCenter.prices.conflict', 'The price list changed meanwhile. Close and try again.') : errorText(reason))
    } finally {
      setBusy(false)
    }
  }
  const save = () => run(() => api.updatePrices({ expected_revision: revision, upserts: [{ entry_key: entryKey, pricing: pricing as never, ...(note.trim() ? { note: note.trim() } : {}) }] }))
  const remove = () => run(() => api.updatePrices({ expected_revision: revision, deletes: [entryKey] }))
  const free = () => {
    if (mode === 'unit') setUnit({ ...unit, amount: '0' })
    else { setMode('tokens'); setTokens({ mode: 'tokens', input: '0', output: '0', cacheInput: '' }) }
  }

  return (
    <Modal title={t('aiCenter.prices.editTitle', 'Price of {{model}}', { model: label })} onClose={onClose} busy={busy} testId="price-dialog">
      <div className="flex flex-col gap-3">
        <p className="break-all text-xs" style={{ color: 'var(--cp-muted)' }}>{entryKey}</p>
        <Segmented
          label={t('aiCenter.prices.mode', 'Priced by')}
          value={mode}
          onChange={setMode}
          options={[
            { value: 'tokens', label: t('aiCenter.prices.mode.tokens', 'Tokens') },
            { value: 'unit', label: t('aiCenter.prices.mode.unit', 'Unit') },
            { value: 'json', label: t('aiCenter.prices.mode.json', 'Advanced') },
          ]}
        />
        {mode === 'tokens' && (
          <>
            <p className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.tokensHint', 'AI Credit per million tokens; 0 is free. Leave cached input empty when the model has no separate rate for it.')}</p>
            <div className="flex flex-col gap-2 sm:flex-row">
              <RateInput testId="price-input" label={t('aiCenter.prices.input', 'Input')} value={tokens.input} onChange={(input) => setTokens({ ...tokens, input })} />
              <RateInput testId="price-output" label={t('aiCenter.prices.output', 'Output')} value={tokens.output} onChange={(output) => setTokens({ ...tokens, output })} />
              <RateInput testId="price-cache" label={t('aiCenter.prices.cacheInput', 'Cached input')} value={tokens.cacheInput} onChange={(cacheInput) => setTokens({ ...tokens, cacheInput })} />
            </div>
          </>
        )}
        {mode === 'unit' && (
          <div className="flex flex-col gap-2 sm:flex-row">
            <RateInput testId="price-amount" label={t('aiCenter.prices.amount', 'AI Credit')} value={unit.amount} onChange={(amount) => setUnit({ ...unit, amount })} />
            <label className="flex flex-1 flex-col gap-1 text-sm">
              <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.unitLabel', 'Unit')}</span>
              <select data-testid="price-unit" value={unit.unit} onChange={(event) => setUnit({ ...unit, unit: event.target.value as PriceUnit })} className="min-h-10 rounded-lg px-3" style={fieldStyle}>
                {PRICE_UNITS.map((item) => <option key={item} value={item}>{unitLabel(item)}</option>)}
              </select>
            </label>
          </div>
        )}
        {mode === 'json' && (
          <label className="flex flex-col gap-1 text-sm">
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.prices.jsonHint', 'The full price in the Provider price format (tiers, rules, add-ons); currency CREDIT, token rates per million tokens.')}</span>
            <textarea data-testid="price-json" value={json} onChange={(event) => setJson(event.target.value)} rows={10} className="rounded-lg px-3 py-2 font-mono text-xs" style={{ ...fieldStyle, borderColor: jsonError ? 'var(--cp-danger)' : 'var(--cp-border)' }} />
          </label>
        )}
        <label className="flex flex-col gap-1 text-sm">
          <span style={{ color: 'var(--cp-text)' }}>{t('aiCenter.prices.note', 'Note (optional)')}</span>
          <input value={note} onChange={(event) => setNote(event.target.value)} maxLength={500} className="min-h-10 rounded-lg px-3" style={fieldStyle} />
        </label>
        {error && <p className="text-xs" data-testid="price-error" style={{ color: 'var(--cp-danger)' }}>{error}</p>}
      </div>
      <div className="mt-5 flex flex-col-reverse gap-2 sm:flex-row sm:flex-wrap sm:justify-end">
        {entry && <button type="button" data-testid="price-delete" disabled={busy} onClick={() => void remove()} className="min-h-11 rounded-lg px-4 text-sm sm:mr-auto" style={buttonStyle.danger}>{t('aiCenter.prices.delete', 'Remove price')}</button>}
        <button type="button" disabled={busy} onClick={free} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('aiCenter.prices.makeFree', 'Make free')}</button>
        <button type="button" onClick={onClose} disabled={busy} className="min-h-11 rounded-lg px-4 text-sm" style={buttonStyle.secondary}>{t('common.cancel', 'Cancel')}</button>
        <button type="button" data-testid="price-save" disabled={busy || !pricing || jsonError} onClick={() => void save()} className="min-h-11 rounded-lg px-4 text-sm font-medium disabled:opacity-50" style={buttonStyle.primary}>{t('aiCenter.prices.save', 'Save price')}</button>
      </div>
    </Modal>
  )
}
