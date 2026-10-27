/* The writing bar (手写笔 TODO D3): shown while the pen or the eraser is the tool — pen / eraser, colours, widths, the
 * pressure switch, the canvas touch lock, and "done". What it sets is the user's own preference (`ui:ink`). */

import { Check, Eraser, Hand, PenLine } from 'lucide-react'
import { useAiwsText } from '../useAiwsText'
import { INK_COLORS, INK_WIDTHS, type InkPrefs } from './prefs'
import type { InkTool } from './input'

export function InkBar({ tool, prefs, onTool, onPrefs, onDone }: {
  tool: InkTool
  prefs: InkPrefs
  onTool: (tool: InkTool) => void
  onPrefs: (patch: Partial<InkPrefs>) => void
  onDone: () => void
}) {
  const { t } = useAiwsText()
  return (
    <div className="aiws-panel aiws-ink-bar" role="toolbar" aria-label={t('aiworkspace.canvas.ink.bar')} data-testid="aiws-ink-bar">
      <button type="button" className="aiws-tool" aria-pressed={tool === 'pen'} aria-label={t('aiworkspace.canvas.ink.pen')} title={t('aiworkspace.canvas.ink.penTip')} data-testid="aiws-ink-pen" onClick={() => onTool('pen')}><PenLine size={18} /></button>
      <button type="button" className="aiws-tool" aria-pressed={tool === 'eraser'} aria-label={t('aiworkspace.canvas.ink.eraser')} title={t('aiworkspace.canvas.ink.eraserTip')} data-testid="aiws-ink-eraser" onClick={() => onTool('eraser')}><Eraser size={18} /></button>
      <span className="aiws-ink-sep" aria-hidden="true" />
      {INK_COLORS.map((color) => (
        <button key={color} type="button" className={`aiws-ink-swatch${prefs.color === color ? ' is-on' : ''}`} style={{ background: color }} aria-pressed={prefs.color === color}
          aria-label={t('aiworkspace.canvas.ink.color', { color })} data-testid={`aiws-ink-color-${color.slice(1)}`} onClick={() => { onPrefs({ color }); onTool('pen') }} />
      ))}
      <span className="aiws-ink-sep" aria-hidden="true" />
      {INK_WIDTHS.map((width, i) => (
        <button key={width} type="button" className="aiws-tool aiws-ink-width" aria-pressed={prefs.width === width} aria-label={t(['aiworkspace.canvas.ink.widthFine', 'aiworkspace.canvas.ink.widthMedium', 'aiworkspace.canvas.ink.widthBold'][i])}
          data-testid={`aiws-ink-width-${width}`} onClick={() => { onPrefs({ width }); onTool('pen') }}>
          <span className="aiws-ink-dot" style={{ width: 3 + width * 1.5, height: 3 + width * 1.5 }} />
        </button>
      ))}
      <span className="aiws-ink-sep" aria-hidden="true" />
      <label className="aiws-ink-switch" title={t('aiworkspace.canvas.ink.pressureTip')}>
        <input type="checkbox" checked={prefs.pressure} data-testid="aiws-ink-pressure" onChange={(event) => onPrefs({ pressure: event.target.checked })} />{t('aiworkspace.canvas.ink.pressure')}
      </label>
      <button type="button" className="aiws-tool" aria-pressed={prefs.touchLock} aria-label={t('aiworkspace.canvas.ink.touchLock')} title={t('aiworkspace.canvas.ink.touchLockTip')} data-testid="aiws-ink-touch-lock"
        onClick={() => onPrefs({ touchLock: !prefs.touchLock })}><Hand size={18} />{prefs.touchLock && <span className="aiws-ink-locked" aria-hidden="true" />}</button>
      <button type="button" className="aiws-tool" aria-label={t('aiworkspace.canvas.ink.done')} title={t('aiworkspace.canvas.ink.doneTip')} data-testid="aiws-ink-done" onClick={onDone}><Check size={18} /></button>
    </div>
  )
}
