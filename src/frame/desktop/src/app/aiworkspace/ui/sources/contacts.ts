import { unwrap } from '../../api/client'
import type { Result, RunView } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'

export type ContactSyncMode = 'auto' | 'manual' | 'once'

export const CONTACT_MODE_LABEL = {
  auto: 'aiworkspace.datasource.contacts.auto',
  manual: 'aiworkspace.datasource.contacts.manual',
  once: 'aiworkspace.datasource.contacts.once',
}

export async function bindContacts(store: WorkspaceStore, entityId: string, mode: ContactSyncMode, requestKey: string,
  target: { create: { parent_id: string; order_key: string; name: string } } | { expect_content_rev: number }) {
  const info = store.session.info()
  return unwrap(await store.session.call<Result<{ run_id?: string; state?: RunView['state'] }>>('source.bind_contacts', {
    workspace_id: info.workspace_id, epoch: info.epoch, entity_id: entityId, mode, idempotency_key: requestKey, ...target,
  }))
}

const ACTIVE_SOURCE_STATES = new Set(['queued', 'running', 'planning', 'validating', 'applying'])

export async function watchSourceRun(store: WorkspaceStore, runId: string, onRun?: (run: RunView) => void) {
  for (;;) {
    const run = await store.session.procGet(runId)
    onRun?.(run)
    if (run.state === 'succeeded') return
    if (!ACTIVE_SOURCE_STATES.has(run.state)) {
      const error = run.result?.status === undefined ? run.result?.error : run.result.status === 'rejected' ? run.result.errors?.[0] : undefined
      throw new Error(error?.detail ?? error?.sub_code ?? error?.code ?? run.state)
    }
    await new Promise((resolve) => window.setTimeout(resolve, 300))
  }
}
