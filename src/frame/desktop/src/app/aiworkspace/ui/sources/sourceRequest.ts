/* Asking for a data source (B4-6; DataSourceRequestDialog.tsx): the request the open workspace's dialog shows, and the
 * action of "协作 ▸ 提数据源需求…". A wish that found external data missing opens it with `openDataSourceRequest` and
 * what the analysis wanted. */

import { aiwsT } from '../../i18n'
import { Emitter } from '../../state/emitter'
import { actionRegistry } from '../framework/actions'

/** What the asker (or a wish's analysis) already knows of the data source; every part can still be edited. */
export interface DataSourceWanted {
  meaning?: string
  origin?: string
  freshness?: string | { max_age_s?: number }
  fields?: Array<string | { name?: string; field_id?: string }>
}

export interface DataSourceRequest {
  /** The wish the data is for (its entity id), or the Block showing it. */
  wishId?: string | null
  blockId?: string | null
  wanted?: DataSourceWanted | null
}

export const sourceRequests = { current: null as DataSourceRequest | null, emitter: new Emitter() }

/** Open the data source request dialog of the open workspace (null closes it). */
export function openDataSourceRequest(request: DataSourceRequest | null = {}) {
  sourceRequests.current = request
  sourceRequests.emitter.emit()
}

actionRegistry.register({
  id: 'collab.request-data', label: () => aiwsT('aiworkspace.sourcereq.action'), phone: true,
  menu: { group: 'collab', order: 80, id: 'request-data' },
  visible: ({ shell }) => !shell.identity.guest && !shell.identity.restricted,
  run: () => openDataSourceRequest(),
})
