import { emojiOnlyCount, loadEmojiGroups, searchEmoji } from '../../src/app/messagehub/conversation/input/emoji.ts'
import { compressWaveform, formatRecordingTime, formatVoiceDuration, pickRecorderType, resampleWaveform, voiceBarCount, voiceFileName } from '../../src/app/messagehub/conversation/media/voice.ts'
import { messageSummaryText } from '../../src/app/messagehub/conversation/history/relations.ts'
import { summarizeMessage, type ProjectionLabels } from '../../src/app/messagehub/api/projection.ts'
import { pendingBubble } from '../../src/app/messagehub/store/outgoing.ts'
import { messageVoice, VOICE_WAVEFORM_MAX, VOICE_WAVEFORM_SAMPLES, type MessageObject } from '../../src/app/messagehub/protocol/msgobj.ts'

function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }

const ME = 'did:user:me', ALICE = 'did:user:alice'
const voiceRef = { role: 'input' as const, label: 'voice-20261019-091500.webm', target: { type: 'data_obj' as const, obj_id: 'cyfile:voice' } }
function voiceMessage(extra: Partial<MessageObject> = {}): MessageObject {
  return { from: ALICE, to: [ME], kind: 'chat', created_at_ms: 1, content: { format: 'text/plain', content: '', refs: [voiceRef] }, voice: { duration_ms: 3200, waveform: [0, 31, 12] }, ...extra }
}

Deno.test('emoji-only text counts up to three emoji and nothing else', () => {
  const cases: Array<[string, number]> = [
    ['👍', 1], ['😂❤️', 2], [' 👍 👍 👍 ', 3], ['👍👍👍👍', 0], ['hi 👍', 0], ['', 0], ['1', 0],
    ['©', 0], ['©️', 1], ['🇨🇳', 1], ['#️⃣', 1], ['👨‍👩‍👧', 1], ['👍🏽', 1], ['❤', 0], ['🏴󠁧󠁢󠁳󠁣󠁴󠁿', 1],
  ]
  for (const [text, count] of cases) equal([text, emojiOnlyCount(text)], [text, count])
})

Deno.test('emoji data loads by category and search ranks exact names first', async () => {
  const groups = await loadEmojiGroups()
  equal(groups.map(group => group.id), ['people', 'nature', 'food', 'activity', 'travel', 'objects', 'symbols', 'flags'])
  equal(groups[0].items[0], { emoji: '😀', name: 'grinning face' })
  const all = groups.flatMap(group => group.items)
  if (all.some(item => !item.emoji || !item.name || item.emoji.includes(' '))) throw Error('malformed emoji line')
  if (new Set(all.map(item => item.emoji)).size !== all.length) throw Error('duplicate emoji')
  const rock = searchEmoji(groups, 'Rock')
  equal(rock[0].name, 'rock')
  if (!rock.some(item => item.name === 'rocket')) throw Error('rocket not found')
  equal(searchEmoji(groups, 'red heart')[0].emoji, '❤️')
  equal(searchEmoji(groups, 'flag china').map(item => item.emoji), ['🇨🇳', '🇭🇰', '🇲🇴'])
  equal(searchEmoji(groups, '   '), [])
})

Deno.test('recorded levels fold into a normalised waveform and stretch for drawing', () => {
  equal(compressWaveform([]), [])
  equal(compressWaveform([0, 0, 0]), [0, 0, 0])
  equal(compressWaveform([0.1, 0.2, 0.05, 0.4]), [8, 16, 4, 31])
  const long = compressWaveform(Array.from({ length: 1000 }, (_, index) => (index % 100) / 100))
  equal([long.length, Math.max(...long), Math.min(...long) >= 0], [VOICE_WAVEFORM_SAMPLES, VOICE_WAVEFORM_MAX, true])
  equal(compressWaveform([0.5, Number.NaN, -1, 0.25], 2), [31, 16])
  equal(resampleWaveform([], 4), [0, 0, 0, 0])
  equal(resampleWaveform([0, 31], 4), [0, 0, 31, 31])
  equal(resampleWaveform([1, 2, 3, 4, 5, 6], 3), [2, 4, 6])
  equal([voiceBarCount(500), voiceBarCount(10_000), voiceBarCount(120_000)], [24, 31, 48])
})

Deno.test('voice times, recorder formats and file names', () => {
  equal([formatVoiceDuration(0), formatVoiceDuration(3200), formatVoiceDuration(65_400), formatVoiceDuration(-5)], ['0:00', '0:03', '1:05', '0:00'])
  equal(formatRecordingTime(1234), '0:01,2')
  equal(pickRecorderType(type => type === 'audio/mp4'), { mimeType: 'audio/mp4', extension: 'm4a' })
  equal(pickRecorderType(type => type.startsWith('audio/webm')), { mimeType: 'audio/webm;codecs=opus', extension: 'webm' })
  equal(pickRecorderType(() => false), null)
  equal(voiceFileName(new Date(2026, 9, 19, 9, 5, 7), 'webm'), 'voice-20261019-090507.webm')
})

Deno.test('messageVoice reads the voice extension leniently and never from an edit', () => {
  equal(messageVoice(voiceMessage()), { duration_ms: 3200, waveform: [0, 31, 12] })
  equal(messageVoice(voiceMessage({ voice: { duration_ms: 1500.4, waveform: [-3, 40, 7.6, 'x'] } as never })), { duration_ms: 1500, waveform: [0, 31, 8, 0] })
  equal(messageVoice(voiceMessage({ voice: { duration_ms: 900 } as never })), { duration_ms: 900, waveform: [] })
  equal(messageVoice(voiceMessage({ voice: { duration_ms: 0, waveform: [] } })), undefined)
  equal(messageVoice(voiceMessage({ voice: [] as never })), undefined)
  equal(messageVoice(voiceMessage({ relates_to: { rel: 'edit', target: 'obj' } })), undefined)
  equal(messageVoice(voiceMessage({ voice: { duration_ms: 1000, waveform: Array.from({ length: 200 }, () => 5) } }))?.waveform.length, VOICE_WAVEFORM_SAMPLES)
})

Deno.test('a voice note summarises as its duration; text still wins', () => {
  equal(messageSummaryText(voiceMessage()), '🎤 0:03')
  equal(messageSummaryText(voiceMessage({ content: { format: 'text/plain', content: 'listen', refs: [voiceRef] } })), 'listen')
  equal(messageSummaryText(voiceMessage({ voice: undefined })), 'voice-20261019-091500.webm')
  const labels: ProjectionLabels = { direct: 'Direct', groupMain: 'Group chat', untitled: 'Untitled', unassigned: 'Unassigned', unassignedDescription: '', you: 'You', previewImage: '[Image]', previewAttachment: '[Attachment]', previewVoice: '[Voice]', previewUnavailable: 'Unavailable' }
  equal(summarizeMessage(voiceMessage(), labels), '[Voice] 0:03')
  equal(summarizeMessage(voiceMessage(), { ...labels, previewVoice: undefined }), '[Attachment] 0:03')
})

Deno.test('the pending bubble of a voice note is already a voice note', () => {
  const file = new File([new Uint8Array(8)], 'voice-20261019-091500.webm', { type: 'audio/webm' })
  const voice = { duration_ms: 2000, waveform: [1, 2, 3] }
  const base = { from: ME, to: [ALICE], kind: 'chat' as const, sessionId: 'dm:alice', senderName: 'You', now: 5 }
  const bubble = pendingBubble({ content: '', attachments: [{ file, voice }], voice, localId: 'l1', submittedAt: 4 }, base, { phase: 'queued' }, true)
  equal([bubble.voice, bubble.content.refs?.length, bubble.created_at_ms], [voice, 1, 4])
  const two = pendingBubble({ content: '', attachments: [{ file, voice }, { file: new File(['x'], 'b.txt') }], voice, localId: 'l2' }, base, undefined, false)
  equal(two.voice, undefined)
})
