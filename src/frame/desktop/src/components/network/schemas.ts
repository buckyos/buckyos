/* ── Dialog contracts of the network components: the future `/sysdlg/*` parameters and results ──
 *
 * Inputs are what a host (or later a third-party App through Sys Dlg) may pass; results carry
 * DIDs and display text only — never invite tokens, session tokens or message bodies.
 */

import { z } from 'zod'
import { FRIEND_MESSAGE_MAX, userTagSchema } from './relation'

export const didSchema = z.string().trim().regex(/^did:[a-z0-9]+:\S+$/i, 'network.error.invalid-input').max(512)

export const lookupKindSchema = z.enum(['user', 'agent', 'group', 'zone', 'device', 'msgtunnel', 'unknown'])
const friendStateSchema = z.enum(['none', 'outgoing_pending', 'incoming_pending', 'friend'])
/** Raw user input: a DID, BNS name, display name, group invite link or HomeStation URL. */
const rawInputSchema = z.string().max(2048)

/** Display information of a chosen person or group, the same shape for every dialog. */
export const pickedPersonSchema = z.object({
  did: didSchema,
  kind: lookupKindSchema,
  name: z.string().max(256),
  avatar: z.string().max(4096).optional(),
  zone: z.string().max(256).optional(),
  isContact: z.boolean(),
  friendState: friendStateSchema.optional(),
})
export type PickedPerson = z.infer<typeof pickedPersonSchema>

const cancelledSchema = z.object({ action: z.literal('cancelled') })

// ── FindPeople ──

export const findPeopleInputSchema = z.object({
  input: rawInputSchema.optional(),
  kinds: z.array(lookupKindSchema).max(7).optional(),
  limit: z.number().int().min(1).max(100).default(30),
  title: z.string().max(80).optional(),
  /** Result-card actions the host offers; choosing one closes the dialog with its id. */
  actions: z.array(z.object({ id: z.string().min(1).max(64), label: z.string().min(1).max(64) })).max(4).default([]),
})
export type FindPeopleInput = z.input<typeof findPeopleInputSchema>
export type FindPeopleParams = z.output<typeof findPeopleInputSchema>

export const findPeopleResultSchema = z.discriminatedUnion('action', [
  z.object({ action: z.literal('invoke'), actionId: z.string(), person: pickedPersonSchema }),
  cancelledSchema,
])
export type FindPeopleResult = z.infer<typeof findPeopleResultSchema>

// ── PeoplePicker ──

export const peoplePickerInputSchema = z.object({
  title: z.string().max(80).optional(),
  confirmLabel: z.string().max(40).optional(),
  mode: z.enum(['single', 'multi']).default('multi'),
  /** Most people that can be selected. */
  limit: z.number().int().min(1).max(500).optional(),
  filter: z.object({
    kinds: z.array(lookupKindSchema).max(7).optional(),
    excludeBlocked: z.boolean().default(true),
    /** Shown but not selectable, with the reason next to them (e.g. “Already in this group”). */
    exclude: z.array(z.object({ did: didSchema, reason: z.string().max(80) })).max(2000).default([]),
    /** Only contacts that carry this tag are listed by default. */
    tag: z.string().max(128).optional(),
  }).default({ excludeBlocked: true, exclude: [] }),
  /** Pick only among these people (e.g. the members of a group); search then filters them locally. */
  candidates: z.array(z.object({ did: didSchema, name: z.string().max(256).optional(), kind: lookupKindSchema.optional() })).max(2000).optional(),
  initialSelection: z.array(didSchema).max(500).default([]),
  /** A DID found by search that is not a contact yet can be saved as one, then selected. */
  allowSaveContact: z.boolean().default(true),
})
export type PeoplePickerInput = z.input<typeof peoplePickerInputSchema>
export type PeoplePickerParams = z.output<typeof peoplePickerInputSchema>

export const peoplePickerResultSchema = z.discriminatedUnion('action', [
  z.object({ action: z.literal('selected'), people: z.array(pickedPersonSchema) }),
  cancelledSchema,
])
export type PeoplePickerResult = z.infer<typeof peoplePickerResultSchema>

// ── AddFriendDialog ──

export const addFriendInputSchema = z.object({
  /** Prefills the search; a DID, name or link. */
  input: rawInputSchema.optional(),
  /** Opens the profile card of this DID directly. */
  did: didSchema.optional(),
  message: z.string().max(FRIEND_MESSAGE_MAX).optional(),
  tags: z.array(userTagSchema).max(20).optional(),
})
export type AddFriendInput = z.input<typeof addFriendInputSchema>
export type AddFriendParams = z.output<typeof addFriendInputSchema>

/** The form inside the profile card. */
export const addFriendFormSchema = z.object({
  message: z.string().trim().max(FRIEND_MESSAGE_MAX, 'network.addFriend.error.messageTooLong'),
  tags: z.array(userTagSchema).max(20),
})
export type AddFriendForm = z.infer<typeof addFriendFormSchema>

export const addFriendOutcomeSchema = z.enum(['friends', 'request_pending', 'request_queued', 'request_failed', 'saved'])
export type AddFriendOutcome = z.infer<typeof addFriendOutcomeSchema>

export const addFriendResultSchema = z.discriminatedUnion('action', [
  z.object({
    action: z.literal('done'),
    outcome: addFriendOutcomeSchema,
    person: pickedPersonSchema,
    tags: z.array(z.string()),
  }),
  cancelledSchema,
])
export type AddFriendResult = z.infer<typeof addFriendResultSchema>

// ── JoinGroupDialog ──

export const joinGroupInputSchema = z.object({
  /** A group DID, BNS name or invite link (`<group_did>?invite=<token>`). */
  input: rawInputSchema.optional(),
})
export type JoinGroupInput = z.input<typeof joinGroupInputSchema>
export type JoinGroupParams = z.output<typeof joinGroupInputSchema>

export const joinGroupFormSchema = z.object({
  input: z.string().trim().min(1, 'network.joinGroup.error.required').max(2048),
})
export type JoinGroupForm = z.infer<typeof joinGroupFormSchema>

export const joinGroupResultSchema = z.discriminatedUnion('action', [
  z.object({ action: z.literal('joined'), groupDid: didSchema, name: z.string().max(256), state: z.enum(['active', 'pending_admin_approval']) }),
  cancelledSchema,
])
export type JoinGroupResult = z.infer<typeof joinGroupResultSchema>
