/* "工作区 ▸ 打印字体…" (打印 TODO §3.5, T6, Q17): which family a print uses for `sans`, `serif` and `mono` — a default
 * font every node has, or one of the font package an administrator downloads. Not set: the system default. Kept in
 * the Workspace's settings (root `config.fonts`), one undoable settings Commit, `manage` on `/` (as the cover). The
 * screen does not change: a print uses these fonts, the Desktop keeps the device's. */

import { useState } from 'react'
import { X } from 'lucide-react'
import { DEFAULT_FONTS, GENERICS, PACK_FONTS, readFontSettings, SYSTEM_DEFAULT, fontChoice, type Generic } from '@print/fonts'
import { describeError } from '../../api/session'
import { useStore } from '../../state/hooks'
import { useWorkspaceSettings } from '../../state/settings'
import { useAiwsText } from '../../useAiwsText'
import { Modal } from '../shell/dialogs'
import { fontSettingsValue } from './fonts'

export function PrintFontsDialog({ onClose }: { onClose: () => void }) {
  const { t } = useAiwsText()
  const store = useStore()
  const settings = useWorkspaceSettings()
  const saved = readFontSettings(settings.value.fonts)
  const [choice, setChoice] = useState<Partial<Record<Generic, string>> | null>(null)
  const current = choice ?? saved
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const locked = settings.writeReason !== null || busy
  const genericLabel: Record<Generic, string> = { sans: t('aiworkspace.print.fonts.sans'), serif: t('aiworkspace.print.fonts.serif'), mono: t('aiworkspace.print.fonts.mono') }

  const write = async () => {
    setBusy(true)
    setError(null)
    try {
      const outcome = await settings.update({ fonts: fontSettingsValue(current) }, t('aiworkspace.print.fonts.commitLabel'))
      if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') {
        const reason = 'errors' in outcome && Array.isArray(outcome.errors) && outcome.errors[0]?.detail ? outcome.errors[0].detail : 'code' in outcome ? String(outcome.code) : outcome.status
        setError(t('aiworkspace.print.fonts.refused', { reason }))
        return
      }
      store.notify('info', t('aiworkspace.print.fonts.saved'))
      onClose()
    } catch (e) {
      setError(t('aiworkspace.print.fonts.failed', { error: describeError(e) }))
    } finally {
      setBusy(false)
    }
  }

  const label = t('aiworkspace.print.fonts.title')
  return (
    <Modal label={label} testId="aiws-print-fonts-dialog" className="aiws-print-dialog" onClose={() => { if (!busy) onClose() }}>
      <div className="aiws-dialog-head"><b>{label}</b><span className="aiws-grow" /><button type="button" className="aiws-tool aiws-tool-small aiws-dialog-close" aria-label={t('aiworkspace.common.close')} title={t('aiworkspace.common.close')} disabled={busy} onClick={onClose}><X size={16} aria-hidden="true" /></button></div>
      <div className="aiws-muted aiws-print-hint">{t('aiworkspace.print.fonts.explain')}</div>
      <div className="aiws-print-fields">
        {GENERICS.map((g) => {
          const value = current[g] ?? ''
          const unknown = value && !fontChoice(value)
          return (
            <label key={g} className="aiws-print-field">{genericLabel[g]}
              <select data-testid={`aiws-print-font-${g}`} value={value} disabled={locked} onChange={(event) => setChoice({ ...current, [g]: event.target.value || undefined })}>
                <option value="">{t('aiworkspace.print.fonts.systemDefault', { family: SYSTEM_DEFAULT[g] })}</option>
                <optgroup label={t('aiworkspace.print.fonts.defaultGroup')}>
                  {DEFAULT_FONTS.filter((f) => f.generic === g).map((f) => <option key={f.family} value={f.family}>{f.family}</option>)}
                </optgroup>
                <optgroup label={t('aiworkspace.print.fonts.packGroup')}>
                  {PACK_FONTS.filter((f) => f.generic === g).map((f) => <option key={f.family} value={f.family}>{f.family}</option>)}
                </optgroup>
                {unknown && <option value={value} disabled>{t('aiworkspace.print.fonts.unknownOption', { family: value })}</option>}
              </select>
            </label>
          )
        })}
      </div>
      <div className="aiws-muted aiws-print-note">{t('aiworkspace.print.fonts.packHint')}</div>
      {settings.writeReason && <div className="aiws-warning" role="status" data-testid="aiws-print-fonts-reason">{settings.writeReason}</div>}
      {error && <div className="aiws-error" role="alert" data-testid="aiws-print-fonts-error">{error}</div>}
      <div className="aiws-dialog-actions">
        <span className="aiws-grow" />
        <button type="button" disabled={busy} onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
        <button type="button" className="is-primary" data-testid="aiws-print-fonts-save" disabled={locked || choice === null} onClick={() => { void write() }}>{t('aiworkspace.common.save')}</button>
      </div>
    </Modal>
  )
}
