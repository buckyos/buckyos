import { bucket } from 'buckyos'
import { ContentError } from './codec.ts'
export const APP_ID = 'text-editor.buckyos.bns.did'
/** Buckets the editor declares in `dapp_meta/app.json` (app_data); they hold its own data, never user files. */
export type AppBucket = 'buffers' | 'recovery'
export type BucketApi = Pick<bucket.BucketClient, 'putBytes' | 'fetchObject' | 'listObjects' | 'delete'>
export interface StoredObject { key: string; size: number; createdAt: number }
export const isMissingObject = (error: unknown): boolean => error instanceof bucket.BucketError && error.code === 'no_such_key'
/** `user/<APP_ID>/<user>/{buffers,recovery}` of the signed-in user. */
export class AppData {
  client: BucketApi; user: string
  constructor(client: BucketApi, user: string) { this.client = client; this.user = user }
  bucketId(name: AppBucket): string { return bucket.userBucketId(APP_ID, this.user, name) }
  async put(name: AppBucket, key: string, bytes: Uint8Array, contentType = 'application/octet-stream'): Promise<void> {
    await this.client.putBytes(this.bucketId(name), key, bytes, { contentType })
  }
  async read(name: AppBucket, key: string, maxSize: number): Promise<Uint8Array> {
    const response = await this.client.fetchObject(this.bucketId(name), key)
    if (Number(response.headers.get('content-length') ?? 0) > maxSize) { await response.body?.cancel(); throw new ContentError('too-large') }
    const bytes = new Uint8Array(await response.arrayBuffer())
    if (bytes.length > maxSize) throw new ContentError('too-large')
    return bytes
  }
  async list(name: AppBucket, prefix = ''): Promise<StoredObject[]> {
    const out: StoredObject[] = []; let startAfter: string | undefined
    do {
      const page = await this.client.listObjects(this.bucketId(name), { prefix, start_after: startAfter, limit: 1000 })
      out.push(...page.items.map(item => ({ key: item.key, size: item.size, createdAt: item.created_at })))
      startAfter = page.next_start_after
    } while (startAfter)
    return out
  }
  /** Unbinds `key`; a key that is already gone is fine. */
  async remove(name: AppBucket, key: string): Promise<void> { await this.client.delete(this.bucketId(name), key) }
}
