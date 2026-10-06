import type { DocumentStore } from './documentStore.ts'
export const APP_ID = 'text-editor.buckyos.bns.did'
export function appDataPath(user: string): string {
  if (!/^[a-zA-Z0-9_.@-]+$/.test(user) || user === '.' || user === '..') throw new Error('invalid-user')
  return `cyfs:///home/${user}/.local/share/${APP_ID}`
}
export async function initializeAppData(store: DocumentStore, user: string): Promise<string> {
  const root = appDataPath(user)
  for (const path of [`.local`, `.local/share`, `.local/share/${APP_ID}`, `.local/share/${APP_ID}/buffers`, `.local/share/${APP_ID}/recovery`]) {
    await store.mkdir(`cyfs:///home/${user}/${path}`)
  }
  return root
}
