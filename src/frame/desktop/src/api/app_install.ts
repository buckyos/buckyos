import { callRpc } from './rpc'

export interface InstallTaskSnapshot {
  schema_version: number
  task_id: string
  task_phase:
    | 'Promised'
    | 'Accepted'
    | 'Running'
    | 'Waiting'
    | 'Paused'
    | 'Terminal'
  task_outcome?: 'Succeeded' | 'Failed' | 'Canceled'
  app_name?: string
  app_version?: string
  app_instance_id?: string
  stage?: string
  progress?: { message?: string }
  available_actions?: string[]
  error?: { code: string; message: string }
}

export interface InstallCancelResult {
  task_id: string
  task_phase: 'Terminal'
  task_outcome: 'Canceled'
  mutation_released: boolean
  cleanup_pending: boolean
}

export async function getInstallTaskStatus(
  taskId: string,
): Promise<InstallTaskSnapshot> {
  const { data, error } = await callRpc<InstallTaskSnapshot>(
    'apps.install.status',
    { task_id: taskId },
  )
  if (error || !data) {
    throw error ?? new Error('Invalid installation task status')
  }
  return data
}

export async function cancelInstallTask(
  taskId: string,
  force = false,
): Promise<InstallCancelResult> {
  const { data, error } = await callRpc<InstallCancelResult>(
    'apps.install.cancel',
    { task_id: taskId, force },
  )
  if (error || !data) {
    throw error ?? new Error('Invalid installation cancellation response')
  }
  return data
}
