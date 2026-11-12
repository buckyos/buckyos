/* A data control as a page of its own (控件机制 TODO G14, Q8; the smallest App mode of 06): its body over the whole
 * window — where a notification or an Agent's link lands on a phone, and what "Open as a page" shows on a computer. The
 * control stays a Block of its canvas; the page only shows it. */

import { ArrowLeft } from 'lucide-react'
import { useEntity } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import { DataFlow } from './DataFlow'

export function ControlPage({ containerId, onBack }: { containerId: string; onBack: () => void }) {
  const { t } = useAiwsText()
  const entity = useEntity(containerId)
  return (
    <div className="aiws-control-page" data-testid={`aiws-control-page-${containerId}`}>
      <div className="aiws-control-page-bar">
        <button type="button" className="aiws-dc-iconbtn" aria-label={t('aiworkspace.datactl.backToCanvas')} title={t('aiworkspace.datactl.backToCanvas')} data-testid="aiws-control-page-back" onClick={onBack}>
          <ArrowLeft size={18} aria-hidden="true" />
        </button>
        <span className="aiws-control-page-title">{entity?.title ?? ''}</span>
      </div>
      {entity ? <DataFlow containerId={containerId} mode="view" host="page" /> : <div className="aiws-dc-empty">{t('aiworkspace.datactl.gone')}</div>}
    </div>
  )
}
