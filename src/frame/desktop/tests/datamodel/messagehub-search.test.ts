import { searchMessages, searchTerms } from '../../src/app/messagehub/conversation/search/model.ts'
import { foldMessageRelations } from '../../src/app/messagehub/conversation/history/relations.ts'
import { entityMatchScore, fuzzyScore, rankEntities } from '../../src/app/messagehub/entitySearch.ts'
import { createSessionSchema, defaultPreferences, latestActiveSession, newSessionFormSchema, NATIVE_CONNECTION_LABEL, presentationSchema, sessionDescription } from '../../src/app/messagehub/sessionModel.ts'
import { senderTone } from '../../src/app/messagehub/senderTone.ts'
import type { MessageObject } from '../../src/app/messagehub/protocol/msgobj.ts'
import type { Session } from '../../src/app/messagehub/types.ts'

const ME = 'did:user:me', BOB = 'did:user:bob'
const now = 1_780_000_000_000
function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }
const chat = (id: string, text: string, extra: Partial<MessageObject> = {}): MessageObject => ({ from: BOB, to: [ME], kind: 'chat', created_at_ms: now + Number(id.replace(/\D/g, '')), content: { content: text }, ui_message_id: id, ...extra })
const session = (id: string, lastActiveAt: number, extra: Partial<Session> = {}): Session => ({ id, ownerDid: ME, entityId: BOB, type: 'chat', title: 'Direct', binding: { kind: 'native', targetDid: BOB }, origin: 'manual', lifecycle: 'active', lastActiveAt, createdAt: now, unreadCount: 0, shared: { title: '', description: '', updatedAt: now }, members: {}, ...extra })

Deno.test('a query is words that must all be present; case and width do not matter', () => {
  equal(searchTerms('  Onboarding   FLOW flow '), ['onboarding', 'flow'])
  equal(searchTerms('ＡＢＣ'), ['abc'])
  const rows = [chat('m1', 'The onboarding flow is on staging'), chat('m2', 'Flow chart attached'), chat('m3', '发布计划已更新')]
  equal(searchMessages(rows, 'flow').map(hit => hit.id), ['m1', 'm2'])
  equal(searchMessages(rows, 'FLOW onboarding').map(hit => hit.id), ['m1'])
  equal(searchMessages(rows, '计划').map(hit => hit.id), ['m3'])
  equal(searchMessages(rows, '   '), [])
  equal(searchMessages(rows, 'missing'), [])
})

Deno.test('search reads what a row shows: edits and attachment names, never status rows, the action log, unreadable or redacted messages', () => {
  const edited = foldMessageRelations([
    chat('m1', 'first draft', { ui_record: { msgId: 'obj-1' } }),
    { ...chat('m2', 'final wording'), from: BOB, relates_to: { rel: 'edit', target: 'obj-1' } },
  ])
  equal(searchMessages(edited, 'final').map(hit => hit.id), ['m1'])
  equal(searchMessages(edited, 'draft'), [])
  const rows = [
    chat('m3', '', { content: { content: '', refs: [{ role: 'input', label: 'budget-2026.xlsx', target: { type: 'data_obj', obj_id: 'cyfile:a' } }] } }),
    chat('m4', 'Bob joined', { kind: 'event', content: { content: 'Bob joined budget review', machine: { intent: 'buckyos.action_log', data: { schema_version: 1, action: 'entity.member_joined' } } } }),
    chat('m5', 'typing budget', { ui_item_kind: 'status' }),
    chat('m6', 'budget unavailable', { ui_unavailable: true }),
  ]
  equal(searchMessages(rows, 'budget').map(hit => hit.id), ['m3'])
  const redacted = foldMessageRelations([
    chat('m7', 'secret budget', { ui_record: { msgId: 'obj-7' } }),
    { ...chat('m8', ''), relates_to: { rel: 'redact', target: 'obj-7' } },
  ])
  equal(redacted.length, 1)
  equal(searchMessages(redacted, 'budget'), [])
})

Deno.test('fuzzy name matching ranks whole, prefix, inner and in-order matches, and drops scattered letters', () => {
  const whole = fuzzyScore('alice', 'Alice'), prefix = fuzzyScore('ali', 'Alice Chen'), inner = fuzzyScore('chen', 'Alice Chen'), ordered = fuzzyScore('alc', 'Alice Chen')
  equal([whole > prefix, prefix > inner, inner > ordered, ordered > 0], [true, true, true, true])
  equal(fuzzyScore('ac', 'A long and winding conversation'), 0)
  equal(fuzzyScore('x', 'Alice'), 0)
  equal(fuzzyScore('', 'Alice') > 0, true)
})

Deno.test('entities match by name fuzzily and by DID as a substring; every word must match; ties keep the list order', () => {
  const entities = [
    { id: 'did:bns:bob', name: 'Bob Zhang' },
    { id: 'did:bns:alice', name: 'Alice Chen' },
    { id: 'did:bns:assistant.alice', name: 'Bucky Assistant' },
    { id: 'did:bns:carol', name: 'Carol Wang' },
  ]
  equal(rankEntities(entities, '').map(entity => entity.name), ['Bob Zhang', 'Alice Chen', 'Bucky Assistant', 'Carol Wang'])
  equal(rankEntities(entities, 'alice').map(entity => entity.name), ['Alice Chen', 'Bucky Assistant'])
  equal(rankEntities(entities, 'did:bns:carol').map(entity => entity.name), ['Carol Wang'])
  equal(rankEntities(entities, 'alice chen').map(entity => entity.name), ['Alice Chen'])
  equal(rankEntities(entities, 'zzz'), [])
  // A DID is never matched by scattered letters.
  equal(entityMatchScore('dbc', { id: 'did:bns:carol', name: 'Nobody' }), 0)
})

Deno.test('an entity opens on its most recently active session; archived ones do not count', () => {
  equal(latestActiveSession([session('a', now - 100), session('b', now - 10), session('c', now, { lifecycle: 'archived' })])?.id, 'b')
  equal(latestActiveSession([session('b', now), session('a', now)])?.id, 'a')
  equal(latestActiveSession([session('c', now, { lifecycle: 'archived' })]), null)
  equal(latestActiveSession([]), null)
})

Deno.test('a session is described by the owner first, then by its shared state; the dialog needs a name', () => {
  const shared = session('a', now, { shared: { title: 'T', description: 'Shared words', updatedAt: now } })
  equal(sessionDescription(shared, defaultPreferences), 'Shared words')
  equal(sessionDescription(shared, { ...defaultPreferences, description: 'Mine' }), 'Mine')
  equal(newSessionFormSchema.safeParse({ title: '  ', description: '', connection: 'native' }).success, false)
  equal(newSessionFormSchema.safeParse({ title: 'Plans', description: 'x'.repeat(501), connection: 'native' }).success, false)
  equal(newSessionFormSchema.parse({ title: ' Plans ', description: ' Q3 ', connection: 'native' }), { title: 'Plans', description: 'Q3', connection: 'native' })
  // The default session of an entity is still created without a name or description.
  equal(createSessionSchema.safeParse({ entityId: BOB, title: '', connection: 'native' }).success, true)
  equal(presentationSchema.safeParse({ title: '', description: 'x'.repeat(501), pinned: false, muted: false }).success, false)
  equal(NATIVE_CONNECTION_LABEL, 'Named Message Protocol')
})

Deno.test('a sender keeps one colour', () => {
  equal(senderTone(BOB), senderTone(BOB))
  equal(new Set(Array.from({ length: 40 }, (_, index) => senderTone(`did:user:member-${index}`))).size > 3, true)
})
