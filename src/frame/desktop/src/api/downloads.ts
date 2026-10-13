import { callRpc } from './rpc.ts'
import { isMockRuntime } from '../runtime.ts'

export interface CreateDownloadInput {
  url: string
  /** Name to save as; derived from the response or the URL when absent. */
  filename?: string
}

/**
 * control-panel `download.create`: downloads the URL into the user's
 * `~/Downloads` as a `download/v1` task that shows in the system
 * notifications. Resolves to the task id.
 */
export async function createDownloadTask(input: CreateDownloadInput): Promise<string> {
  const filename = input.filename?.trim() || undefined
  if (isMockRuntime()) {
    const { createMockDownload } = await import('./downloads_mock.ts')
    return createMockDownload({ url: input.url.trim(), filename }).taskId
  }
  const { data, error } = await callRpc<{ task_id?: string }>('download.create', {
    url: input.url.trim(),
    ...(filename ? { filename } : {}),
    idempotency_key: crypto.randomUUID(),
  })
  if (!data?.task_id) throw error instanceof Error ? error : new Error(String(error ?? 'download.create failed'))
  return data.task_id
}

/** `http(s)://host/...`, the only URLs the downloader takes. */
export function isDownloadableUrl(value: string): boolean {
  try {
    const url = new URL(value.trim())
    return (url.protocol === 'http:' || url.protocol === 'https:') && Boolean(url.hostname)
  } catch {
    return false
  }
}
