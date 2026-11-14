/* A data control as a page of its own (控件机制 TODO G14, Q8; the smallest App mode of 06): its body over the whole
 * window — where a notification or an Agent's link lands on a phone, and what "Open as a page" shows on a computer. The
 * control stays a Block of its canvas; the page only shows it. Its bar is the editor's title row (UI 框架 PRD §10.4): ⋯
 * lists the actions the body publishes, the same ones as the Block's menu on a canvas. */

import { ArrowLeft, Ellipsis } from 'lucide-react'
import { useEntity } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { usePublishedActions } from '../../blockui/commands'
import { MenuButton } from '../../shell/popover'
import { DataFlow } from './DataFlow'
import './controlPanel.css'

export function ControlPage({ containerId, onBack }: { containerId: string; onBack: () => void }) {
  const { t } = useAiwsText()
  const entity = useEntity(containerId)
  const published = usePublishedActions(entity ? containerId : null)
  return (
    <div className="aiws-control-page" data-testid={`aiws-control-page-${containerId}`}>
      <div className="aiws-control-page-bar">
        <button type="button" className="aiws-dc-iconbtn" aria-label={t('aiworkspace.datactl.backToCanvas')} title={t('aiworkspace.datactl.backToCanvas')} data-testid="aiws-control-page-back" onClick={onBack}>
          <ArrowLeft size={18} aria-hidden="true" />
        </button>
        <span className="aiws-control-page-title">{entity?.title ?? ''}</span>
        {published.length > 0 && (
          <span className="aiws-control-page-menu">
            <MenuButton label={t('aiworkspace.canvas.near.more')} className="aiws-dc-iconbtn" testId="aiws-control-page-menu" menuTestId="aiws-control-page-menu-list" align="end"
              items={() => published.map((a) => ({ id: `control-${a.id}`, label: a.label, checked: a.checked, disabled: Boolean(a.disabled), reason: a.disabled || null, run: a.run }))}>
              <Ellipsis size={18} aria-hidden="true" />
            </MenuButton>
          </span>
        )}
      </div>
      {entity ? <DataFlow containerId={containerId} mode="view" host="page" /> : <div className="aiws-dc-empty">{t('aiworkspace.datactl.gone')}</div>}
    </div>
  )
}
