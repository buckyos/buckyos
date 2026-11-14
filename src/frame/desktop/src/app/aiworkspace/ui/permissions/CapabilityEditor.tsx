/* Editing what one entry says (§8.3): each of the nine capabilities is allowed, refused or not set; the presets only
 * fill the allowed set, every capability can still be changed afterwards. What a subject may never be allowed (R18) is
 * offered disabled with the reason. Both sets empty means the entry goes. */

import { useEffect, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { X } from 'lucide-react'
import type { Capability } from '../../api/types'
import { useAiwsText } from '../../useAiwsText'
import { Modal } from '../shell/dialogs'
import { useOverlayMounted } from '../shell/popover'
import { usePhone } from '../shell/shellContext'
import { PRESETS } from '../sources/presets'
import { CAPS, applyPreset, capState, withCap, type CapState, type EntryCaps } from './model'

const STATES: readonly CapState[] = ['allow', 'deny', 'unset']

/** One capability's name in words, with its code for those who know the model. */
export function CapName({ cap }: { cap: Capability }) {
  const { t } = useAiwsText()
  return <span title={cap}>{t(`aiworkspace.perm.cap.${cap}`)}</span>
}

/** Allowed and refused capabilities as tags, refused ones in their own colour. */
export function CapTags({ allow, deny, empty }: { allow: readonly Capability[]; deny?: readonly Capability[]; empty?: ReactNode }) {
  const { t } = useAiwsText()
  if (allow.length === 0 && (deny?.length ?? 0) === 0) return <span className="aiws-muted">{empty ?? t('aiworkspace.perm.none')}</span>
  return (
    <span className="aiws-perm-tags">
      {CAPS.filter((c) => allow.includes(c)).map((c) => <span key={`a-${c}`} className="aiws-perm-tag is-allow" data-cap={c} data-state="allow"><CapName cap={c} /></span>)}
      {CAPS.filter((c) => deny?.includes(c)).map((c) => <span key={`d-${c}`} className="aiws-perm-tag is-deny" data-cap={c} data-state="deny" aria-label={t('aiworkspace.perm.deniedCap', { cap: t(`aiworkspace.perm.cap.${c}`) })}><CapName cap={c} /></span>)}
    </span>
  )
}

export function CapabilityEditor({ value, onChange, forbidden = [], locked = [], disabled = false }: {
  value: EntryCaps
  onChange: (next: EntryCaps) => void
  /** Capabilities this subject may not be allowed (they can still be refused). */
  forbidden?: readonly Capability[]
  /** Capabilities this caller may not change at all (`manage` for anyone but the owner, R17). */
  locked?: readonly Capability[]
  disabled?: boolean
}) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-perm-caps" data-testid="aiws-perm-caps">
      <div className="aiws-perm-presets" role="group" aria-label={t('aiworkspace.perm.presets')}>
        <span className="aiws-muted">{t('aiworkspace.perm.presets')}</span>
        {PRESETS.map((p) => (
          <button key={p.id} type="button" data-testid={`aiws-perm-preset-${p.id}`} disabled={disabled || p.caps.every((c) => forbidden.includes(c))}
            onClick={() => onChange(applyPreset(value, p.caps, [...forbidden, ...locked]))}>{p.label}</button>
        ))}
      </div>
      <div className="aiws-perm-cap-grid">
        {CAPS.map((cap) => {
          const state = capState(value, cap)
          const noAllow = forbidden.includes(cap)
          const fixed = locked.includes(cap)
          return (
            <div key={cap} className="aiws-perm-cap-row" data-testid={`aiws-perm-cap-${cap}`} data-state={state}>
              <span className="aiws-perm-cap-name"><CapName cap={cap} /><span className="aiws-muted"> {t(`aiworkspace.perm.capHint.${cap}`)}</span></span>
              <span className="aiws-perm-seg" role="radiogroup" aria-label={t(`aiworkspace.perm.cap.${cap}`)}>
                {STATES.map((s) => {
                  const off = disabled || fixed || (s === 'allow' && noAllow && state !== 'allow')
                  return (
                    <button key={s} type="button" role="radio" aria-checked={state === s} className={`is-${s}`} data-testid={`aiws-perm-cap-${cap}-${s}`} disabled={off}
                      title={fixed ? t('aiworkspace.perm.capOwnerOnly') : s === 'allow' && noAllow ? t('aiworkspace.perm.capForbidden') : undefined}
                      onClick={() => onChange(withCap(value, cap, s))}>{t(`aiworkspace.perm.state.${s}`)}</button>
                  )
                })}
              </span>
            </div>
          )
        })}
      </div>
      {forbidden.length > 0 && forbidden.length < CAPS.length && <div className="aiws-muted" data-testid="aiws-perm-caps-forbidden">{t('aiworkspace.perm.forbiddenNote', { caps: forbidden.map((c) => t(`aiworkspace.perm.cap.${c}`)).join(t('aiworkspace.common.listSeparator')) })}</div>}
    </div>
  )
}

/** A dialog on a computer, a sheet from the bottom on a phone (§8.5); above the whole window wherever it is opened from
 * (a drawer, the side panel, another dialog). */
export function Sheet({ label, testId, onClose, children, wide = false }: { label: string; testId?: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  const phone = usePhone()
  const { t } = useAiwsText()
  useOverlayMounted(phone)
  useEffect(() => {
    if (!phone) return
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [phone, onClose])
  const head = (
    <div className="aiws-dialog-head aiws-perm-sheet-head">
      <b>{label}</b>
      <span className="aiws-grow" />
      <button type="button" className="aiws-tool" aria-label={t('aiworkspace.common.close')} data-testid="aiws-perm-sheet-close" onClick={onClose}><X size={18} /></button>
    </div>
  )
  const host = (document.querySelector('.aiws-root') as HTMLElement | null) ?? document.body
  if (!phone) return createPortal(<Modal label={label} testId={testId} onClose={onClose} className={`aiws-perm-dialog${wide ? ' is-wide' : ''}`}>{head}{children}</Modal>, host)
  return createPortal(
    <div className="aiws-perm-sheet-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-perm-sheet" role="dialog" aria-modal="true" aria-label={label} data-testid={testId}>
        <div className="aiws-perm-sheet-grip" aria-hidden="true" />
        {head}
        <div className="aiws-perm-sheet-body">{children}</div>
      </div>
    </div>,
    host,
  )
}
