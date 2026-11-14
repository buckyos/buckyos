/* Conflict / refusal of one value: both values stay visible and my input is kept until I choose (design §6.5). */

import type { WorkspaceStore } from '../../state/store'
import { useAiwsText } from '../../useAiwsText'

interface ConflictBoxProps {
  store: WorkspaceStore
  editId: string
  theirs: string | null
  mine: string | null
  detail: string
  onKeepMine?: () => void
  onEditAgain?: () => void
}

export function ConflictBox({ store, editId, theirs, mine, detail, onKeepMine, onEditAgain }: ConflictBoxProps) {
  const { t, tNode } = useAiwsText()
  return (
    <div className="aiws-conflict" role="alertdialog" aria-label={t('aiworkspace.data.conflict.label')} data-testid="aiws-conflict">
      <div className="aiws-conflict-detail">{detail}</div>
      {theirs !== null && <div>{tNode('aiworkspace.data.conflict.theirs', { value: <b data-testid="aiws-conflict-theirs">{theirs}</b> })}</div>}
      {mine !== null && <div>{tNode('aiworkspace.data.conflict.mine', { value: <b data-testid="aiws-conflict-mine">{mine}</b> })}</div>}
      <div className="aiws-conflict-actions">
        {onKeepMine && <button type="button" data-testid="aiws-conflict-keep-mine" onClick={onKeepMine}>{t('aiworkspace.data.conflict.keepMine')}</button>}
        {onEditAgain && <button type="button" onClick={onEditAgain}>{t('aiworkspace.data.conflict.editAgain')}</button>}
        <button type="button" data-testid="aiws-conflict-discard" onClick={() => { void store.dismissEdit(editId) }}>{theirs !== null ? t('aiworkspace.data.conflict.useTheirs') : t('aiworkspace.data.conflict.discardMine')}</button>
      </div>
    </div>
  )
}
