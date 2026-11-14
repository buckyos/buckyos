/* The home's small dialogs: a workspace's details (what the card no longer shows: its ID and commit sequence) and
 * the confirmation before deleting one. */

import type { WorkspaceListItem } from '../../api/types'
import type { PreparedHint } from '../../offline/holder'
import { useAiwsText } from '../../useAiwsText'
import { Modal } from '../shell/dialogs'
import { ROLE_KEY, roleOf } from './model'

export function DetailsDialog({ workspace, me, prepared, onClose }: { workspace: WorkspaceListItem; me: string | null; prepared: PreparedHint | undefined; onClose: () => void }) {
  const { t, dateTime, list } = useAiwsText()
  const rows: [string, string, string][] = [
    ['id', t('aiworkspace.home.details.id'), workspace.workspace_id],
    ['seq', t('aiworkspace.home.details.seq'), String(workspace.head_seq)],
    ['role', t('aiworkspace.home.details.role'), t(ROLE_KEY[roleOf(workspace, me)])],
    ...(workspace.owner_principal ? [['owner', t('aiworkspace.home.details.owner'), workspace.owner_principal] as [string, string, string]] : []),
    ...(workspace.updated_at ? [['modified', t('aiworkspace.home.details.modified'), dateTime(workspace.updated_at)] as [string, string, string]] : []),
    ['capabilities', t('aiworkspace.home.details.capabilities'), list(workspace.capabilities)],
    ['offline', t('aiworkspace.home.details.offline'), prepared ? t('aiworkspace.home.details.preparedAt', { at: dateTime(prepared.prepared_at) }) : t('aiworkspace.home.details.notPrepared')],
  ]
  return (
    <Modal label={t('aiworkspace.home.details.title', { name: workspace.title })} testId="aiws-home-details-dialog" onClose={onClose}>
      <div className="aiws-dialog-head"><b>{t('aiworkspace.home.details.title', { name: workspace.title })}</b><span className="aiws-grow" /><button type="button" className="aiws-link" autoFocus onClick={onClose}>{t('aiworkspace.common.close')}</button></div>
      <dl className="aiws-home-details">
        {rows.map(([key, label, value]) => <div key={key} data-testid={`aiws-home-details-${key}`}><dt>{label}</dt><dd>{value}</dd></div>)}
      </dl>
    </Modal>
  )
}

export function DeleteDialog({ workspace, busy, onConfirm, onClose }: { workspace: WorkspaceListItem; busy: boolean; onConfirm: () => void; onClose: () => void }) {
  const { t } = useAiwsText()
  return (
    <Modal label={t('aiworkspace.home.delete.title', { name: workspace.title })} testId="aiws-home-delete-dialog" onClose={onClose}>
      <b>{t('aiworkspace.home.delete.title', { name: workspace.title })}</b>
      <div>{t('aiworkspace.home.delete.body')}</div>
      <div className="aiws-dialog-actions">
        <button type="button" className="is-danger" data-testid="aiws-delete-confirm" disabled={busy} onClick={onConfirm}>{t('aiworkspace.core.list.confirmDelete')}</button>
        <button type="button" autoFocus onClick={onClose}>{t('aiworkspace.common.cancel')}</button>
      </div>
    </Modal>
  )
}
