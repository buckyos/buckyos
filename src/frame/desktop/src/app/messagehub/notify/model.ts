/**
 * System Notify as MessageHub reads it: the wire shapes of `notify.*`
 * (`buckyos-api/src/msg_notify.rs`) and the rules a card follows. Pure, so
 * the Deno datamodel tests use it directly.
 */
import {
  NOTIFY_CHOICE_CONTROL_ID,
  NOTIFY_TEXT_CONTROL_ID,
  type HumanPromptOption,
  type HumanTaskPrompt,
  type NotifyCategory,
  type NotifyKind,
  type NotifyLevel,
  type NotifySource,
  type NotifyStatus,
} from '../protocol/msgobj'

export const NOTIFY_ACTION_ANSWER = 'answer'
export const NOTIFY_ACTION_DISMISS = 'dismiss'
/** TaskMgr error codes the answer path tells apart (`task_mgr_client.ts`). */
export const TASK_ERR_ALREADY_COMPLETED = 'task_already_completed'
export const TASK_ERR_REVISION_CONFLICT = 'revision_conflict'

/** What the user (or someone on their behalf) submitted for an `input`. */
export interface NotifyResult {
  /** `answer` or `dismiss`. */
  action: string
  values?: Record<string, unknown>
  submitted_by?: string
  submitted_at?: number
  /** `<user>|<app>` that committed it, from TaskMgr. */
  committed_by?: string
}

/** The record of a notification in the user's notification session. */
export interface NotifyRecordRef {
  owner: string
  session_id: string
  record_id: string
  msg_id: string
  /** The system notify sender. */
  from: string
}

export interface NotifyDelivery {
  session_id: string
  platform: string
  to: string
  to_session?: string
  msg_id?: string
  ok: boolean
  reason?: string
}

/** `notify.get` / `notify.list` item: the only source of a card's state. */
export interface NotifyView {
  notify_id: string
  url: string
  to_user: string
  kind: NotifyKind
  category: NotifyCategory
  level: NotifyLevel
  status: NotifyStatus
  prompt: HumanTaskPrompt
  source: NotifySource
  created_at_ms: number
  expires_at_ms?: number
  closed_at_ms?: number
  closed_reason?: string
  /** Who acknowledged a `notice` on the user's behalf, when not the user. */
  acked_by?: string
  task_id?: string
  result?: NotifyResult
  home?: NotifyRecordRef
  deliveries?: NotifyDelivery[]
}

/** What the user does with a card: an option, a text, or dismiss. */
export interface NotifyAnswer {
  dismiss?: boolean
  option?: string
  text?: string
}

/** `open` and `read` can still change; every other status is final. */
export function isNotifyClosed(status: NotifyStatus): boolean {
  return status !== 'open' && status !== 'read'
}

/** The options of the prompt's `option` choice. */
export function promptOptions(prompt: HumanTaskPrompt): HumanPromptOption[] {
  for (const control of prompt.controls ?? []) if (control.kind === 'choice' && control.id === NOTIFY_CHOICE_CONTROL_ID) return control.options ?? []
  return []
}

/** The prompt's `text` input, when it takes free text. */
export function promptTextInput(prompt: HumanTaskPrompt) {
  for (const control of prompt.controls ?? []) if (control.kind === 'text_input' && control.id === NOTIFY_TEXT_CONTROL_ID) return control
  return undefined
}

/** The task result of an answer: `TaskDataResult::Human` exactly as `NotifyAnswer::to_task_result` builds it. */
export function notifyTaskResult(answer: NotifyAnswer, submittedBy: string, submittedAtMs: number) {
  const values: Record<string, string> = {}
  if (answer.option) values[NOTIFY_CHOICE_CONTROL_ID] = answer.option
  if (answer.text?.trim()) values[NOTIFY_TEXT_CONTROL_ID] = answer.text
  return {
    kind: 'human' as const,
    action: answer.dismiss ? NOTIFY_ACTION_DISMISS : NOTIFY_ACTION_ANSWER,
    ...(Object.keys(values).length ? { values } : {}),
    submitted_by: submittedBy,
    submitted_at: submittedAtMs,
  }
}

/** Checks an answer against the prompt of an `input` (`NotifyAnswer::validate`); `null` when it is acceptable. */
export function validateNotifyAnswer(answer: NotifyAnswer, prompt: HumanTaskPrompt): 'unknown_option' | 'text_not_accepted' | 'empty' | null {
  if (answer.dismiss) return null
  if (answer.option !== undefined && !promptOptions(prompt).some(option => option.value === answer.option)) return 'unknown_option'
  const hasText = !!answer.text?.trim()
  if (hasText && !promptTextInput(prompt)) return 'text_not_accepted'
  if (answer.option === undefined && !hasText) return 'empty'
  return null
}

export function resultOption(result: NotifyResult | undefined): string | undefined {
  const value = result?.values?.[NOTIFY_CHOICE_CONTROL_ID]
  return typeof value === 'string' && value ? value : undefined
}

export function resultText(result: NotifyResult | undefined): string | undefined {
  const value = result?.values?.[NOTIFY_TEXT_CONTROL_ID]
  return typeof value === 'string' && value.trim() ? value : undefined
}

/** The App part of `committed_by` (`<user>|<app>`). */
export function committedApp(result: NotifyResult | undefined): string | undefined {
  const value = result?.committed_by
  const index = value?.indexOf('|') ?? -1
  return value && index >= 0 ? value.slice(index + 1) || undefined : undefined
}

/** Client-side filter of a notification session. */
export type NotifyFilter = 'all' | 'pending' | 'done'
export const NOTIFY_FILTERS: readonly NotifyFilter[] = ['all', 'pending', 'done']

/** Whether a card with this status (`undefined` while unknown) shows under the filter. */
export function notifyFilterMatches(status: NotifyStatus | undefined, filter: NotifyFilter): boolean {
  if (filter === 'all') return true
  if (!status) return false
  return filter === 'done' ? isNotifyClosed(status) : !isNotifyClosed(status)
}
