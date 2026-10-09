import { APP_ID } from '../fs/appData.ts'
export interface Settings {
  schema_version: 1; fontSize: number; tabSize: number; insertSpaces: boolean; wordWrap: 'auto' | 'on' | 'off'
  lineNumbers: boolean; renderWhitespace: boolean; autoReloadClean: boolean; restoreSession: boolean
  closeKeepsChanges: boolean; recoveryRetentionDays: { discarded: number; takenOver: number }
  trimTrailingWhitespaceOnSave: boolean; ensureFinalNewline: boolean
}
export const defaultSettings: Settings = { schema_version: 1, fontSize: 14, tabSize: 4, insertSpaces: true, wordWrap: 'auto', lineNumbers: true, renderWhitespace: false, autoReloadClean: true, restoreSession: true, closeKeepsChanges: false, recoveryRetentionDays: { discarded: 7, takenOver: 7 }, trimTrailingWhitespaceOnSave: false, ensureFinalNewline: false }
export class SettingsStore {
  private client: { get(key: string): Promise<{ value: string }>; set(key: string, value: string): Promise<unknown> }
  private path: string
  constructor(client: SettingsStore['client'], user: string) { this.client = client; this.path = `users/${user}/apps/${APP_ID}/settings` }
  async load(): Promise<Settings> {
    try {
      const value = JSON.parse((await this.client.get(this.path)).value)
      return { ...defaultSettings, ...value, fontSize: Math.max(8, Math.min(40, Number(value.fontSize) || 14)), tabSize: Math.max(1, Math.min(8, Number(value.tabSize) || 4)), recoveryRetentionDays: { ...defaultSettings.recoveryRetentionDays, ...value.recoveryRetentionDays } }
    } catch { return { ...defaultSettings } }
  }
  async save(value: Settings): Promise<void> { await this.client.set(this.path, JSON.stringify(value)) }
}
