import { formatInviteLink, groupErrorText, parseGroupNotice, parseInviteLink, sortSessionParticipants, withinWindow } from '../../src/app/messagehub/groupModel.ts'
import { displayedContent, foldMessageRelations, isHiddenRelationMessage, mentionsViewer, messageObjId, messageRelations, ownReactionId } from '../../src/app/messagehub/conversation/history/relations.ts'
import { applyMention, collectMentions, matchMentionCandidates, mentionQueryAt } from '../../src/app/messagehub/conversation/input/mentions.ts'
import type { MessageObject } from '../../src/app/messagehub/protocol/msgobj.ts'

const GROUP = 'did:buckyos:group:team'
const ME = 'did:user:me', BOB = 'did:user:bob', AGENT = 'did:agent:mine'
const now = 1_780_000_000_000
function equal(actual: unknown, expected: unknown) { if (JSON.stringify(actual) !== JSON.stringify(expected)) throw Error(`Expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}`) }
const t = (key: string, _fallback?: string, variables?: Record<string, string | number>) => variables ? `${key}:${JSON.stringify(variables)}` : key

function notice(action: string, data: Record<string, unknown>, from = BOB): MessageObject {
  return { from, to: [ME], kind: 'operation', created_at_ms: now, content: { content: action, machine: { intent: 'buckyos.group_invitation', data: { group_did: GROUP, action, data: data as Record<string, never> } } } }
}

Deno.test('group notices carry the invitation state, the agent member, transfer and session details', () => {
  const auto = parseGroupNotice(notice('invite', { invite_id: 'inv-1', role: 'admin', expires_at_ms: now + 1, state: 'active' }))
  equal(auto?.invitation, { groupDid: GROUP, inviteId: 'inv-1', role: 'admin', expiresAt: now + 1, inviterDid: BOB, state: 'active' })
  const forAgent = parseGroupNotice(notice('invite', { invite_id: 'inv-2', role: 'member', state: 'invited', member_did: AGENT }))
  equal([forAgent?.memberDid, forAgent?.invitation?.memberDid, forAgent?.invitation?.state], [AGENT, AGENT, 'invited'])
  equal(parseGroupNotice(notice('invite', { invite_id: 'inv-3', role: 'owner', state: 'bogus' }))?.invitation?.state, undefined)
  equal(parseGroupNotice(notice('pending_approval', { member_did: BOB, invited_by: ME })), { groupDid: GROUP, action: 'pending_approval', memberDid: BOB, invitedBy: ME })
  equal(parseGroupNotice(notice('owner_transfer', { transfer_id: 'tr-1', expires_at_ms: now + 5 })), { groupDid: GROUP, action: 'owner_transfer', transferId: 'tr-1', expiresAt: now + 5 })
  equal(parseGroupNotice(notice('session_invite', { session_id: 'reading', title: 'Reading list' })), { groupDid: GROUP, action: 'session_invite', sessionId: 'reading', sessionTitle: 'Reading list' })
  equal(parseGroupNotice({ ...notice('invite', {}), content: { content: 'x' } }), null)
})

Deno.test('group error texts know the post-proof reason codes and keep unknown ones verbatim', () => {
  for (const code of ['invitation-mismatch', 'transfer-mismatch', 'agent-owner-required', 'invite-required', 'join-not-allowed', 'member-not-pending', 'revision-conflict']) equal(groupErrorText(t, Error(`rejected: ${code}`)), `messagehub.groupError.${code}`)
  equal(groupErrorText(t, Error('revision-conflict:abc')), 'messagehub.groupError.revision-conflict')
  equal(groupErrorText(t, Error('member-proof-required')), 'messagehub.groupError.generic:{"reason":"member-proof-required"}')
  equal(groupErrorText(t, Error('agent_observer')), 'messagehub.reason.agent_observer')
})

Deno.test('invite links round-trip through formatting and parsing', () => {
  const link = formatInviteLink(GROUP, 'tok/en+1')
  equal(link, `${GROUP}?invite=tok%2Fen%2B1`)
  equal(parseInviteLink(` ${link} `), { groupDid: GROUP, invite: 'tok/en+1' })
  equal(parseInviteLink(GROUP), { groupDid: GROUP })
  equal(parseInviteLink('https://example.com?invite=x'), null)
  equal(parseInviteLink(''), null)
  equal([withinWindow(undefined, 0, now), withinWindow(1000, now - 500, now), withinWindow(1000, now - 5000, now), withinWindow(0, now, now)], [true, true, false, false])
})

function msg(id: string, from: string, content: string, extra: Partial<MessageObject> = {}, at = now): MessageObject {
  return { from, to: [GROUP], kind: 'group_msg', created_at_ms: at, ui_message_id: id, ui_sender_name: from, content: { format: 'text/plain', content }, ...extra }
}

Deno.test('relation messages fold into their targets: latest own edit, redaction, grouped reactions, reply quotes', () => {
  const original = msg('m1', ME, 'hello', { ui_record: { msgId: 'obj-1' } })
  const edit1 = msg('e1', ME, 'hello there', { relates_to: { rel: 'edit', target: 'obj-1' } }, now + 1)
  const edit2 = msg('e2', ME, 'hello again', { relates_to: { rel: 'edit', target: 'obj-1' } }, now + 2)
  const foreignEdit = msg('e3', BOB, 'hijack', { relates_to: { rel: 'edit', target: 'obj-1' } }, now + 3)
  const react1 = msg('r1', BOB, '👍', { relates_to: { rel: 'reaction', target: 'obj-1', key: '👍' } }, now + 4)
  const react2 = msg('r2', ME, '👍', { relates_to: { rel: 'reaction', target: 'obj-1', key: '👍' } }, now + 5)
  const react3 = msg('r3', BOB, '👍', { relates_to: { rel: 'reaction', target: 'obj-1', key: '👍' } }, now + 6)
  const reply = msg('m2', BOB, 'hi back', { relates_to: { rel: 'thread', target: 'obj-1' } }, now + 7)
  const other = msg('m3', BOB, 'bye', {}, now + 8)
  const redact = msg('x1', BOB, '', { relates_to: { rel: 'redact', target: 'm3' } }, now + 9)
  const orphan = msg('m4', ME, 'reply to nothing', { relates_to: { rel: 'thread', target: 'missing' } }, now + 10)
  const folded = foldMessageRelations([original, edit1, edit2, foreignEdit, react1, react2, react3, reply, other, redact, orphan])
  equal(folded.map(item => item.ui_message_id), ['m1', 'm2', 'm3', 'm4'])
  equal(messageRelations(folded[0]), { edited: { content: { format: 'text/plain', content: 'hello again' }, at: now + 2, edits: [{ id: 'e1', at: now + 1, content: { format: 'text/plain', content: 'hello there' } }, { id: 'e2', at: now + 2, content: { format: 'text/plain', content: 'hello again' } }] }, reactions: [{ key: '👍', dids: [BOB, ME], messages: { [BOB]: 'r1', [ME]: 'r2' } }] })
  equal(ownReactionId(folded[0], ME, '👍'), 'r2')
  equal(ownReactionId(folded[0], ME, '❤️'), undefined)
  equal(displayedContent(folded[0]), 'hello again')
  equal(messageRelations(folded[1])?.replyTo, { id: 'obj-1', from: ME, senderName: ME, content: 'hello again', found: true })
  equal(messageRelations(folded[2])?.redacted, { by: BOB, at: now + 9 })
  equal(messageRelations(folded[3])?.replyTo, { id: 'missing', from: '', content: '', found: false })
  equal([isHiddenRelationMessage(edit1), isHiddenRelationMessage(reply), isHiddenRelationMessage(other)], [true, false, false])
  equal(messageObjId(original), 'obj-1')
  equal(messageObjId(msg('local', ME, 'x', { ui_sent_msg_id: 'obj-9' })), 'obj-9')
  equal(foldMessageRelations([other]), [other])
})

Deno.test('cancelling a reaction is a redact of the reaction message; reacting again afterwards counts anew', () => {
  const original = msg('m1', BOB, 'hello', { ui_record: { msgId: 'obj-1' } })
  const mine = msg('r1', ME, '👍', { relates_to: { rel: 'reaction', target: 'obj-1', key: '👍' } }, now + 1)
  const bobs = msg('r2', BOB, '👍', { relates_to: { rel: 'reaction', target: 'obj-1', key: '👍' } }, now + 2)
  const heart = msg('r3', ME, '❤️', { relates_to: { rel: 'reaction', target: 'obj-1', key: '❤️' } }, now + 3)
  const cancelMine = msg('x1', ME, '', { relates_to: { rel: 'redact', target: 'r1' } }, now + 4)
  const cancelHeart = msg('x2', ME, '', { relates_to: { rel: 'redact', target: 'r3' } }, now + 5)
  const foreignCancel = msg('x3', BOB, '', { relates_to: { rel: 'redact', target: 'r2' } }, now + 6)
  const again = msg('r4', ME, '👍', { relates_to: { rel: 'reaction', target: 'obj-1', key: '👍' } }, now + 7)
  const [afterCancel] = foldMessageRelations([original, mine, bobs, heart, cancelMine, cancelHeart])
  equal(messageRelations(afterCancel), { reactions: [{ key: '👍', dids: [BOB], messages: { [BOB]: 'r2' } }] })
  equal(ownReactionId(afterCancel, ME, '👍'), undefined)
  const [afterAll] = foldMessageRelations([original, mine, bobs, heart, cancelMine, cancelHeart, foreignCancel])
  equal(messageRelations(afterAll), undefined)
  equal(messageRelations(foldMessageRelations([original, mine, cancelMine, again])[0]), { reactions: [{ key: '👍', dids: [ME], messages: { [ME]: 'r4' } }] })
  // The redact of a reaction never counts as a redaction of the target.
  equal(messageRelations(afterAll)?.redacted, undefined)
})

Deno.test('mentions are structured, never empty, and only count names still in the text', () => {
  const picked = [{ did: BOB, name: 'Bob' }, { did: AGENT, name: 'Agent' }]
  equal(collectMentions('hi @Bob and @all', picked, true), { dids: [BOB], all: true })
  equal(collectMentions('hi @Bob', picked, true), { dids: [BOB] })
  equal(collectMentions('hi all', picked, false), undefined)
  equal(collectMentions('@all', picked, false), undefined)
  equal(mentionsViewer(msg('m', BOB, 'x', { mentions: { dids: [ME] } }), ME), true)
  equal(mentionsViewer(msg('m', BOB, 'x', { mentions: { all: true } }), ME), true)
  equal(mentionsViewer(msg('m', BOB, 'x', { mentions: { dids: [BOB] } }), ME), false)
  equal(mentionsViewer(msg('m', BOB, 'x'), ME), false)
})

Deno.test('typing @ looks up a mention at the caret, never inside a word', () => {
  equal(mentionQueryAt('hi @bo', 6), { start: 3, end: 6, query: 'bo' })
  equal(mentionQueryAt('@', 1), { start: 0, end: 1, query: '' })
  // The caret inside a word: the query stops at the caret, the replacement covers the word.
  equal(mentionQueryAt('hi @bob there', 5), { start: 3, end: 7, query: 'b' })
  equal(mentionQueryAt('（@张', 3), { start: 1, end: 3, query: '张' })
  equal(mentionQueryAt('mail@example.com', 8), null)
  equal(mentionQueryAt('hi @bob there', 13), null)
  equal(mentionQueryAt('no mention', 10), null)
})

Deno.test('mention suggestions rank prefix, word and substring matches, with @all first while it matches', () => {
  const people = [{ did: 'a', name: 'Alice Chen' }, { did: 'b', name: 'Bob Zhang' }, { did: 'c', name: 'Cathy' }, { did: 'z', name: '张三' }]
  const ids = (query: string, all: boolean) => matchMentionCandidates(people, query, all).map(option => option === 'all' ? 'all' : option.did)
  equal(ids('', true), ['all', 'a', 'b', 'c', 'z'])
  equal(ids('', false), ['a', 'b', 'c', 'z'])
  equal(ids('A', true), ['all', 'a', 'b', 'c'])
  equal(ids('ch', true), ['a'])
  equal(ids('张', true), ['z'])
  equal(ids('xyz', true), [])
  equal(matchMentionCandidates(Array.from({ length: 12 }, (_, index) => ({ did: `${index}`, name: `Member ${index}` })), 'm', false).length, 8)
})

Deno.test('a picked mention replaces the typed @word and leaves the caret after one space', () => {
  const bob = { did: BOB, name: 'Bob Zhang' }
  equal(applyMention('hi @bo', { start: 3, end: 6, query: 'bo' }, bob), { text: 'hi @Bob Zhang ', caret: 14 })
  equal(applyMention('@b there', { start: 0, end: 2, query: 'b' }, bob), { text: '@Bob Zhang there', caret: 11 })
  equal(applyMention('@bob x', { start: 0, end: 4, query: 'bo' }, bob), { text: '@Bob Zhang x', caret: 11 })
  equal(applyMention('@a', { start: 0, end: 2, query: 'a' }, 'all'), { text: '@all ', caret: 5 })
})

Deno.test('session participants list the owner first and pending guests last', () => {
  const sorted = sortSessionParticipants([
    { did: 'g-invited', kind: 'guest', state: 'invited' },
    { did: 'guest', kind: 'guest', state: 'included' },
    { did: 'member', kind: 'group_member', role: 'member', state: 'included' },
    { did: 'owner', kind: 'group_member', role: 'owner', state: 'included' },
    { did: 'admin', kind: 'group_member', role: 'admin', state: 'included' },
  ])
  equal(sorted.map(item => item.did), ['owner', 'admin', 'member', 'guest', 'g-invited'])
})
