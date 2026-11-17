/**
 * The colour a person keeps everywhere in MessageHub (list avatar, group avatar, group sender name),
 * so pictures without an image still tell people apart. Drawn from the shell's app tones, never the
 * semantic ones: a person is not an error or a warning.
 */
const SENDER_TONES = [
  'var(--cp-app-blue)',
  'var(--cp-app-green)',
  'var(--cp-app-amber)',
  'var(--cp-app-red)',
  'var(--cp-app-violet)',
  'color-mix(in srgb, var(--cp-app-blue) 50%, var(--cp-app-green))',
  'color-mix(in srgb, var(--cp-app-red) 50%, var(--cp-app-violet))',
]

export function senderTone(did: string): string {
  let hash = 0
  for (let index = 0; index < did.length; index++) hash = (hash * 31 + did.charCodeAt(index)) >>> 0
  return SENDER_TONES[hash % SENDER_TONES.length]
}

/** The tone as text: a name in a person's colour, readable on bubbles and panels in both themes. */
export function senderInk(did: string): string {
  return `color-mix(in srgb, ${senderTone(did)} 80%, var(--cp-text))`
}
