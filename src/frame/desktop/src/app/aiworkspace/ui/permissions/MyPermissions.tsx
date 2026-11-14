/* "My permissions" (§8.4, §5.2): what someone without `manage` sees — the groups it belongs to and how, and each
 * capability on the Workspace or one object with where it comes from. Read-only; it follows the permissions' revision. */

import { useAiwsText } from '../../useAiwsText'
import { entityLabel } from '../sources/dataOps'
import { useAcl, useAclRead } from './aclData'
import { DecisionOrder, ExplainTable, JudgmentSummary } from './CheckPanel'
import { TypeIcon } from './icons'
import { useLabels } from './labels'

export interface MyPermissionsProps {
  /** The object; absent: the whole Workspace. */
  entityId?: string | null
}

export function MyPermissions({ entityId = null }: MyPermissionsProps) {
  const { t } = useAiwsText()
  const labels = useLabels()
  const { api } = useAcl()
  const resource = entityId ?? '/'
  const explained = useAclRead(`explain:me:${resource}`, () => api.explain(resource))
  const entity = labels.entity(entityId)
  return (
    <section className="aiws-perm-mine" data-testid="aiws-perm-mine" data-entity-id={entityId ?? ''}>
      <div className="aiws-perm-section-head">
        <TypeIcon entity={entity} />
        <h3>{entityId ? t('aiworkspace.perm.mine.titleEntity', { name: entity ? entityLabel(entity) : entityId }) : t('aiworkspace.perm.mine.title')}</h3>
      </div>
      <p className="aiws-muted">{t('aiworkspace.perm.mine.intro')}</p>
      {explained.error && <div className="aiws-error" role="alert">{explained.error}</div>}
      {explained.loading && !explained.data && <div className="aiws-muted">{t('aiworkspace.perm.loading')}</div>}
      {explained.data && (
        <>
          <JudgmentSummary explained={explained.data} labels={labels} />
          <ExplainTable explained={explained.data} labels={labels} />
        </>
      )}
      <DecisionOrder />
    </section>
  )
}
