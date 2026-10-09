export class LocalStore {
  private db: Promise<IDBDatabase>
  constructor(user: string) {
    this.db = new Promise((resolve, reject) => {
      const request = indexedDB.open(`buckyos.text-editor.${user}`, 1)
      request.onupgradeneeded = () => request.result.createObjectStore('records')
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => reject(request.error)
    })
  }
  async get<T>(key: string): Promise<T | undefined> {
    const db = await this.db
    return new Promise((resolve, reject) => { const r = db.transaction('records').objectStore('records').get(key); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error) })
  }
  async put(key: string, value: unknown): Promise<void> { await this.write(key, value, false) }
  async delete(key: string): Promise<void> { await this.write(key, undefined, true) }
  private async write(key: string, value: unknown, remove: boolean): Promise<void> {
    const db = await this.db
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction('records', 'readwrite'); const store = tx.objectStore('records')
      if (remove) store.delete(key); else store.put(value, key)
      tx.oncomplete = () => resolve(); tx.onerror = () => reject(tx.error); tx.onabort = () => reject(tx.error)
    })
  }
  async entries<T>(prefix: string): Promise<Array<[string, T]>> {
    const db = await this.db
    return new Promise((resolve, reject) => {
      const out: Array<[string, T]> = []; const r = db.transaction('records').objectStore('records').openCursor()
      r.onerror = () => reject(r.error)
      r.onsuccess = () => { const c = r.result; if (!c) return resolve(out); if (String(c.key).startsWith(prefix)) out.push([String(c.key), c.value]); c.continue() }
    })
  }
}
export async function holdLock(name: string): Promise<(() => Promise<void>) | undefined> {
  if (!navigator.locks) return async () => {}
  return new Promise((resolve, reject) => {
    const finished = navigator.locks.request(name, { ifAvailable: true }, async lock => {
      if (!lock) { resolve(undefined); return }
      await new Promise<void>(release => resolve(async () => { release(); await finished }))
    })
    void finished.catch(reject)
  })
}
