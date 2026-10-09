/* loro-crdt is loaded through its `web` build (see the alias in vite.config.ts): one explicit init. */

import * as loroWeb from 'loro-crdt/web'
import loroWasmUrl from 'loro-crdt/web/loro_wasm_bg.wasm?url'

let ready: Promise<void> | null = null

/** The `web` build's default export is its init function; its typings do not declare it. */
type LoroInit = (input: { module_or_path: string }) => Promise<unknown>

export function loadLoro(): Promise<void> {
  if (!ready) {
    const init = (loroWeb as unknown as { default: LoroInit }).default
    ready = init({ module_or_path: loroWasmUrl }).then(() => undefined)
  }
  return ready
}
