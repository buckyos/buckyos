/* Clipboard and drag carriers for desktop Apps (剪贴板规范 §3): one way to write and read a transfer from a real
 * event, from a toolbar or menu command, and a fallback for browsers that fire no clipboard event for a shortcut. */
import {
  readClipboard, snapshotDataTransfer, transferReader, writeClipboard, writeDataTransfer,
  type TransferReader, type TransferRepresentations,
} from 'buckyos/transfer'

/** Writes one transfer in a `copy`, `cut` or `dragstart` event (call `preventDefault()` on a clipboard event after). */
export function writeTransferEvent(dt: DataTransfer | null | undefined, representations: TransferRepresentations): boolean {
  if (!dt) return false
  writeDataTransfer(dt, representations)
  return true
}

/** Reads a `paste` or `drop` event. Call it before the handler returns or awaits anything. */
export const readTransferEvent = (dt: DataTransfer | null | undefined): TransferReader => transferReader(snapshotDataTransfer(dt))

export type CommandCopyResult =
  | { ok: true; formats: string[]; /** false when a Web standard format (`text/plain`, `text/html`) could not be written. */ complete: boolean }
  | { ok: false }

/** A toolbar or menu Copy / Cut (inside the user's click): writes through the async clipboard. The BuckyOS standard
 * content and the transfer handle also travel embedded in `text/html`, so a browser without Web custom formats still
 * carries them; `text/uri-list` cannot be written this way. */
export async function copyWithCommand(representations: TransferRepresentations): Promise<CommandCopyResult> {
  try {
    const formats = await writeClipboard(representations)
    const standard = ['text/plain', 'text/html'].filter((f) => f in representations)
    return { ok: true, formats, complete: standard.every((f) => formats.includes(f)) }
  } catch {
    return { ok: false }
  }
}

export type CommandPasteResult = { ok: true; reader: TransferReader } | { ok: false; reason: 'denied' | 'unsupported' }

/** A toolbar or menu Paste (inside the user's click): reads the system clipboard. When the browser refuses, the caller
 * keeps focus and points to the paste shortcut. */
export async function readClipboardCommand(): Promise<CommandPasteResult> {
  if (!globalThis.navigator?.clipboard) return { ok: false, reason: 'unsupported' }
  try {
    return { ok: true, reader: transferReader(await readClipboard()) }
  } catch {
    return { ok: false, reason: 'denied' }
  }
}

// Some browsers (Safari with nothing selected) fire no clipboard event for Ctrl/⌘ C, X or V.
const lastEvent = { copy: 0, cut: 0, paste: 0 }
if (typeof document !== 'undefined') {
  for (const kind of ['copy', 'cut', 'paste'] as const) document.addEventListener(kind, () => { lastEvent[kind] = performance.now() }, true)
}

/** From a shortcut's `keydown` (without `preventDefault()`): runs `fallback` when the browser fires no `copy`, `cut`
 * or `paste` event for it, so the shortcut still works through the async clipboard. */
export function afterShortcut(kind: 'copy' | 'cut' | 'paste', fallback: () => void): void {
  const at = performance.now()
  window.setTimeout(() => { if (lastEvent[kind] < at) fallback() }, 0)
}

/** The element an App's clipboard handler may claim: not an editor, field or dialog (they keep the platform's own
 * clipboard behaviour, §3.3). */
export function isEditableTarget(target: EventTarget | null): boolean {
  const el = target instanceof Element ? target : null
  return !!el?.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"], [contenteditable="plaintext-only"], [role="dialog"], [data-role="editor"]')
}
