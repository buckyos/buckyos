/** Colours a sender keeps across a conversation, so pictures without an image still tell people apart. */
const SENDER_TONES = [
  'var(--cp-accent)',
  'var(--cp-success)',
  'var(--cp-warning)',
  'var(--cp-danger)',
  'color-mix(in srgb, var(--cp-accent) 55%, var(--cp-danger))',
  'color-mix(in srgb, var(--cp-accent) 55%, var(--cp-success))',
  'color-mix(in srgb, var(--cp-warning) 55%, var(--cp-danger))',
]

export function senderTone(did: string): string {
  let hash = 0
  for (let index = 0; index < did.length; index++) hash = (hash * 31 + did.charCodeAt(index)) >>> 0
  return SENDER_TONES[hash % SENDER_TONES.length]
}
