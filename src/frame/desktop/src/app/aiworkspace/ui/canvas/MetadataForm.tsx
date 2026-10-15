/* A Block's link, tooltip and data (draw.io 导入 TODO §2.6): Cell `config.link` (a web address or `#<entity id>`),
 * `config.tooltip` (shown on hover) and `config.data` (string keys and values, draw.io's "edit data"). */

import { useState } from 'react'
import type { Json } from '../../api/types'

export function MetadataForm({ config, readOnly, onChange }: { config: Record<string, Json>; readOnly: boolean; onChange: (patch: Record<string, Json | null>, label: string) => void }) {
  const link = typeof config.link === 'string' ? config.link : ''
  const tooltip = typeof config.tooltip === 'string' ? config.tooltip : ''
  const data = config.data && typeof config.data === 'object' && !Array.isArray(config.data) ? (config.data as Record<string, Json>) : {}
  const [draft, setDraft] = useState({ key: '', value: '' })
  const setData = (next: Record<string, Json>) => onChange({ data: Object.keys(next).length ? next : null }, '数据')
  return (
    <details className="aiws-details" data-testid="aiws-metadata" open={Boolean(link || tooltip || Object.keys(data).length)}>
      <summary>链接、提示与数据</summary>
      <label className="aiws-inline-form">链接
        <input type="text" placeholder="https://… 或 #对象 id" defaultValue={link} key={`link:${link}`} disabled={readOnly} data-testid="aiws-meta-link"
          onBlur={(e) => { const v = e.target.value.trim(); if (v !== link) onChange({ link: v || null }, v ? '链接' : '清除链接') }} />
      </label>
      <label className="aiws-inline-form">提示
        <input type="text" placeholder="悬停时显示" defaultValue={tooltip} key={`tip:${tooltip}`} disabled={readOnly} data-testid="aiws-meta-tooltip"
          onBlur={(e) => { const v = e.target.value.trim(); if (v !== tooltip) onChange({ tooltip: v || null }, v ? '提示' : '清除提示') }} />
      </label>
      <table className="aiws-meta-data" data-testid="aiws-meta-data">
        <tbody>
          {Object.entries(data).map(([k, v]) => (
            <tr key={k}>
              <th>{k}</th>
              <td><input type="text" defaultValue={String(v)} key={`${k}:${String(v)}`} disabled={readOnly} aria-label={k} onBlur={(e) => { if (e.target.value !== String(v)) setData({ ...data, [k]: e.target.value }) }} /></td>
              <td><button type="button" className="aiws-link" disabled={readOnly} onClick={() => { const { [k]: _gone, ...rest } = data; void _gone; setData(rest) }}>删除</button></td>
            </tr>
          ))}
        </tbody>
      </table>
      {!readOnly && (
        <div className="aiws-inline-form">
          <input aria-label="数据名" placeholder="名称" value={draft.key} onChange={(e) => setDraft({ ...draft, key: e.target.value })} data-testid="aiws-meta-key" />
          <input aria-label="数据值" placeholder="值" value={draft.value} onChange={(e) => setDraft({ ...draft, value: e.target.value })} data-testid="aiws-meta-value" />
          <button type="button" data-testid="aiws-meta-add" disabled={!/^[A-Za-z_一-鿿][\w.\-一-鿿]*$/.test(draft.key.trim())}
            onClick={() => { setData({ ...data, [draft.key.trim()]: draft.value }); setDraft({ key: '', value: '' }) }}>添加</button>
        </div>
      )}
    </details>
  )
}
