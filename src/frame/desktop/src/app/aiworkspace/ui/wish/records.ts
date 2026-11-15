/* A wish's history (许愿格交互升级 §3.4): which record of a stage is the current one, and the one rule that turns a
 * record's material into the text its reader sees, in the reader's language. The service stores facts only (counts,
 * codes, names, objects); the model's own summary is shown as it wrote it. */

import { runErrorText } from '../../api/session'
import type { WishHistory, WishRecord, WishRecordWarning, WishStage } from '../../api/types'
import type { AiwsText } from '../../i18n'

export const STAGES: readonly WishStage[] = ['analyze', 'code', 'run']

export function stageRecords(history: WishHistory | undefined, stage: WishStage): WishRecord[] {
  return history?.[stage] ?? []
}

/** All records, newest first. */
export function allRecords(history: WishHistory | undefined): WishRecord[] {
  return STAGES.flatMap((stage) => stageRecords(history, stage)).sort((a, b) => (a.at < b.at ? 1 : a.at > b.at ? -1 : 0))
}

/** The newest record of any stage. */
export function latestRecord(history: WishHistory | undefined): WishRecord | undefined {
  return allRecords(history)[0]
}

/** The record of a stage that is in effect: the newest successful one not rolled back (a rollback record is none). */
export function currentRecord(records: readonly WishRecord[]): WishRecord | undefined {
  return records.find((r) => r.ok && !r.rolled_back && r.action !== 'rollback')
}

export function findRecord(history: WishHistory | undefined, id: string | undefined): WishRecord | undefined {
  return id ? allRecords(history).find((r) => r.id === id) : undefined
}

/** The digest as people read it (宏 a3f9c2). */
export const shortDigest = (digest: string | undefined) => (digest ?? '').slice(0, 6)

const ACTION_LABEL: Record<WishRecord['action'], string> = {
  analyze: 'aiworkspace.wishcard.action.analyze', code: 'aiworkspace.wishcard.action.code', repair: 'aiworkspace.wishcard.action.repair',
  run: 'aiworkspace.wishcard.action.run', rollback: 'aiworkspace.wishcard.action.rollback',
}

export function actionText(action: WishRecord['action'], text: AiwsText): string {
  return text.t(ACTION_LABEL[action])
}

/** A duration for people: 48 秒, 6 分 12 秒, 1 小时 5 分. */
export function durationText(ms: number, text: AiwsText): string {
  const total = Math.max(0, Math.round(ms / 1000))
  const h = Math.floor(total / 3600)
  const m = Math.floor((total % 3600) / 60)
  const s = total % 60
  if (h > 0) return text.t('aiworkspace.wishcard.duration.hm', { h, m })
  if (m > 0) return text.t('aiworkspace.wishcard.duration.ms', { m, s })
  return text.t('aiworkspace.wishcard.duration.s', { s })
}

/** The record's start as a short date and time (11-14 15:02). */
export function whenText(at: string, text: AiwsText): string {
  const date = new Date(at)
  if (Number.isNaN(date.getTime())) return at
  try {
    return new Intl.DateTimeFormat(text.locale, { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }).format(date)
  } catch {
    return text.dateTime(date)
  }
}

/** What a failed record failed of, with the inputs it could not find named. */
export function failureText(record: WishRecord, text: AiwsText): string {
  const error = record.error
  if (!error) return text.t('aiworkspace.wishcard.record.failedUnknown')
  const missing = (error.missing_inputs ?? []).map((m) => m.label || m.message).filter(Boolean)
  if (missing.length) return text.t('aiworkspace.wishcard.error.needsInputNamed', { inputs: text.list(missing) })
  return runErrorText({ code: error.code, sub_code: error.sub_code, detail: error.detail, missing_inputs: error.missing_inputs }) ?? error.code
}

const NOTICE_LABEL: Record<string, string> = {
  check_failed: 'aiworkspace.wishcard.notice.checkFailed', check_not_run: 'aiworkspace.wishcard.notice.checkNotRun',
  uncited_numbers: 'aiworkspace.wishcard.notice.uncitedNumbers', overwrote_manual: 'aiworkspace.wishcard.notice.overwroteManual',
  overwrote_manual_cells: 'aiworkspace.wishcard.notice.overwroteManualCells', deleted_rows: 'aiworkspace.wishcard.notice.deletedRows',
  deleted_field: 'aiworkspace.wishcard.notice.deletedField', changed_type: 'aiworkspace.wishcard.notice.changedType',
  changed_structure: 'aiworkspace.wishcard.notice.changedStructure', not_produced: 'aiworkspace.wishcard.notice.notProduced',
  recreated_column: 'aiworkspace.wishcard.notice.recreatedColumn', discarded_edits: 'aiworkspace.wishcard.notice.discardedEdits',
}

/** A warning of a record in the reader's language: the model's text as it wrote it, the service's by its code. */
export function warningText(w: WishRecordWarning, text: AiwsText): string {
  if (typeof w === 'string') return w
  const key = NOTICE_LABEL[w.code]
  if (!key) return w.code
  const vars: Record<string, string | number> = {}
  for (const [k, v] of Object.entries(w)) {
    if (k === 'code') continue
    vars[k] = Array.isArray(v) ? text.list(v.map(String)) : typeof v === 'number' ? v : String(v ?? '')
  }
  return text.t(key, vars)
}

/** The parts of a record's sentence (§3.4): "11-14 15:02 根据 `愿望` 执行“分析需求”（48 秒 · 输入 12,340 / 输出 1,205 token ·
 * gpt-6-astra）成功：要生成 3 项结果。" — the warnings go first in the outcome and are shown in bold. The one rule every
 * record is said by; the basis and the names are links the view draws. */
export interface RecordParts {
  when: string
  action: string
  /** Duration, then tokens and the model where the record has them (a run of a macro only when it used AI). */
  metrics: string
  /** 成功 / 失败. */
  status: string
  /** The outcome's own sentence (the warnings go before it). */
  outcome: string
  warnings: string[]
}

export function describeRecord(record: WishRecord, text: AiwsText): RecordParts {
  const metrics = [durationText(record.duration_ms, text)]
  const usage = record.usage
  if (usage && (usage.input_tokens !== undefined || usage.output_tokens !== undefined)) {
    metrics.push(text.t('aiworkspace.wishcard.record.tokens', { input: text.num(usage.input_tokens ?? 0), output: text.num(usage.output_tokens ?? 0) }))
  }
  if (usage?.model) metrics.push(usage.model)
  const s = record.summary ?? {}
  let outcome: string
  if (!record.ok) outcome = failureText(record, text)
  else if (record.action === 'rollback') outcome = text.t('aiworkspace.wishcard.record.rolledBackDone')
  else if (record.stage === 'analyze') {
    const results = s.results ?? record.outputs.analysis?.results ?? 0
    const checks = s.checks ?? record.outputs.analysis?.checks ?? 0
    outcome = text.t('aiworkspace.wishcard.record.analyzed', { count: results, checks })
  } else if (record.stage === 'code') {
    const trial = s.trial ?? {}
    outcome = text.t('aiworkspace.wishcard.record.coded', { count: trial.results ?? 0, passed: trial.checks_passed ?? 0, total: trial.checks_total ?? 0 })
  } else {
    outcome = text.t('aiworkspace.wishcard.record.ran', { created: s.created ?? 0, updated: s.updated ?? 0 })
  }
  return {
    when: whenText(record.at, text), action: actionText(record.action, text), metrics: metrics.join(text.t('aiworkspace.wishcard.record.metricSeparator')),
    status: text.t(record.ok ? 'aiworkspace.wishcard.record.ok' : 'aiworkspace.wishcard.record.failed'), outcome, warnings: (record.warnings ?? []).map((w) => warningText(w, text)),
  }
}
