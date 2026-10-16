/* A Block's link, tooltip and data (draw.io 导入 TODO §2.6): Cell `config.link` (a web address or `#<entity id>`),
 * `config.tooltip` (shown on hover) and `config.data` (string keys and values, draw.io's "edit data"). */

import { useState } from 'react'
import type { Json } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'

export function MetadataForm({ config, readOnly, onChange }: { config: Record<string, Json>; readOnly: boolean; onChange: (patch: Record<string, Json | null>, label: string) => void }) {
  const { t } = useAiwsText()
  const link = typeof config.link === 'string' ? config.link : ''
  const tooltip = typeof config.tooltip === 'string' ? config.tooltip : ''
  const data = config.data && typeof config.data === 'object' && !Array.isArray(config.data) ? (config.data as Record<string, Json>) : {}
  const [draft, setDraft] = useState({ key: '', value: '' })
  const setData = (next: Record<string, Json>) => onChange({ data: Object.keys(next).length ? next : null }, t('aiworkspace.canvas.meta.data'))
  return (
    <details className="aiws-details" data-testid="aiws-metadata" open={Boolean(link || tooltip || Object.keys(data).length)}>
      <summary>{t('aiworkspace.canvas.meta.title')}</summary>
      <label className="aiws-inline-form">{t('aiworkspace.canvas.meta.link')}
        <input type="text" placeholder={t('aiworkspace.canvas.meta.linkPlaceholder')} defaultValue={link} key={`link:${link}`} disabled={readOnly} data-testid="aiws-meta-link"
          onBlur={(e) => { const v = e.target.value.trim(); if (v !== link) onChange({ link: v || null }, t(v ? 'aiworkspace.canvas.meta.link' : 'aiworkspace.canvas.meta.clearLink')) }} />
      </label>
      <label className="aiws-inline-form">{t('aiworkspace.canvas.meta.tooltip')}
        <input type="text" placeholder={t('aiworkspace.canvas.meta.tooltipPlaceholder')} defaultValue={tooltip} key={`tip:${tooltip}`} disabled={readOnly} data-testid="aiws-meta-tooltip"
          onBlur={(e) => { const v = e.target.value.trim(); if (v !== tooltip) onChange({ tooltip: v || null }, t(v ? 'aiworkspace.canvas.meta.tooltip' : 'aiworkspace.canvas.meta.clearTooltip')) }} />
      </label>
      <table className="aiws-meta-data" data-testid="aiws-meta-data">
        <tbody>
          {Object.entries(data).map(([k, v]) => (
            <tr key={k}>
              <th>{k}</th>
              <td><input type="text" defaultValue={String(v)} key={`${k}:${String(v)}`} disabled={readOnly} aria-label={k} onBlur={(e) => { if (e.target.value !== String(v)) setData({ ...data, [k]: e.target.value }) }} /></td>
              <td><button type="button" className="aiws-link" disabled={readOnly} onClick={() => { const { [k]: _gone, ...rest } = data; void _gone; setData(rest) }}>{t('aiworkspace.common.delete')}</button></td>
            </tr>
          ))}
        </tbody>
      </table>
      {!readOnly && (
        <div className="aiws-inline-form">
          <input aria-label={t('aiworkspace.canvas.meta.dataName')} placeholder={t('aiworkspace.canvas.meta.name')} value={draft.key} onChange={(e) => setDraft({ ...draft, key: e.target.value })} data-testid="aiws-meta-key" />
          <input aria-label={t('aiworkspace.canvas.meta.dataValue')} placeholder={t('aiworkspace.canvas.meta.value')} value={draft.value} onChange={(e) => setDraft({ ...draft, value: e.target.value })} data-testid="aiws-meta-value" />
          <button type="button" data-testid="aiws-meta-add" disabled={!/^[A-Za-z_一-鿿][\w.\-一-鿿]*$/.test(draft.key.trim())}
            onClick={() => { setData({ ...data, [draft.key.trim()]: draft.value }); setDraft({ key: '', value: '' }) }}>{t('aiworkspace.canvas.meta.add')}</button>
        </div>
      )}
    </details>
  )
}
