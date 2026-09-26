import type { ApiType } from '../mock/types'

export type JsonObject = Record<string, unknown>
export interface PlaygroundField {
  key: string
  kind: 'text' | 'number' | 'select' | 'boolean' | 'object' | 'array' | 'resource' | 'union' | 'json' | 'checks'
  required?: boolean
  options?: string[]
  min?: number
  max?: number
  step?: number
  fields?: PlaygroundField[]
  item?: PlaygroundField
  variants?: Record<string, PlaygroundField[]>
  discriminator?: string
  initial?: unknown
  accept?: string
  structured?: boolean
  nullable?: boolean
}
export interface PlaygroundApi { method: string; fields: PlaygroundField[] }
export const record = (value: unknown): JsonObject => value !== null && typeof value === 'object' && !Array.isArray(value) ? value as JsonObject : {}
const text = (key: string, required = false): PlaygroundField => ({ key, kind: 'text', required })
const structuredText = (key: string, required = true, nullable = false): PlaygroundField => ({ ...text(key, required), structured: true, nullable })
const num = (key: string, min = 0, max?: number, step = 1): PlaygroundField => ({ key, kind: 'number', min, max, step })
const select = (key: string, options: string[], required = false): PlaygroundField => ({ key, kind: 'select', options, required })
const bool = (key: string): PlaygroundField => ({ key, kind: 'boolean' })
const object = (key: string, fields: PlaygroundField[], required = false): PlaygroundField => ({ key, kind: 'object', fields, required })
const array = (key: string, item: PlaygroundField, required = false): PlaygroundField => ({ key, kind: 'array', item, required })
const resource = (key: string, accept = 'image/*', required = true): PlaygroundField => ({ key, kind: 'resource', accept, required })
const union = (key: string, variants: Record<string, PlaygroundField[]>): PlaygroundField => ({ key, kind: 'union', variants, discriminator: 'type', required: true })
const checks = (key: string, options: string[]): PlaygroundField => ({ key, kind: 'checks', options })
const json = (key: string, required = false): PlaygroundField => ({ key, kind: 'json', required })
const language = (key = 'language') => select(key, ['en', 'zh', 'ja', 'ko', 'fr', 'de', 'es', 'pt', 'ar', 'ru'])
const prompt = text('prompt', true)
const output = object('output', [select('media_type', ['image/png', 'image/jpeg', 'image/webp', 'audio/mpeg', 'audio/wav', 'audio/ogg', 'video/mp4']), select('size', ['512x512', '1024x1024', '1536x1024', '1024x1536']), num('sample_rate', 1), num('fps', 1)])
const aspect = select('aspect_ratio', ['1:1', '16:9', '9:16', '4:3', '3:4', '3:2', '2:3'])
const resolution = (key = 'resolution', required = false) => select(key, ['480p', '720p', '1080p', '4k'], required)
const duration = num('duration_seconds', 0.1, undefined, 0.1)
const bbox = object('bbox', [select('format', ['xywh'], true), select('unit', ['px', 'relative'], true), { ...num('x', 0, undefined, 0.01), required: true }, { ...num('y', 0, undefined, 0.01), required: true }, { ...num('width', 0.01, undefined, 0.01), required: true }, { ...num('height', 0.01, undefined, 0.01), required: true }], true)
const message = object('', [select('role', ['user', 'system', 'assistant', 'developer', 'tool'], true), array('content', union('', {
  text: [text('text', true)], image: [resource('source')], document: [resource('source', 'application/pdf,text/*'), text('title')],
  thinking: [text('summary'), text('text')], tool_use: [text('call_id', true), text('name', true), json('args', true)],
  tool_result: [text('call_id', true), array('content', union('', { text: [text('text', true)], image: [resource('source')], document: [resource('source', 'application/pdf,text/*'), text('title')] }), true), bool('is_error')], provider_state: [object('source', [text('provider_profile_id', true), text('adapter_type', true), text('origin_provider', true), text('origin_model', true)], true), text('provider', true), json('value', true)],
}), true)], true)
const question = union('', {
  boolean: [text('id', true), structuredText('instructions'), object('criteria', [structuredText('true', false), structuredText('false', false)])],
  choice: [text('id', true), structuredText('instructions'), array('options', object('', [text('id', true), structuredText('description', true, true)], true), true)],
  score: [text('id', true), structuredText('instructions'), { ...array('levels', structuredText(''), true), min: 2 }],
})

export const PLAYGROUND_APIS: Record<ApiType, PlaygroundApi> = {
  llm: { method: 'chat.completions.create', fields: [array('messages', message, true), num('temperature', 0, 2, 0.1), num('top_p', 0, 1, 0.05), num('max_output_tokens', 1), num('seed'), array('stop', text('', true)), object('response_format', [select('type', ['text', 'json', 'json_object', 'json_schema'], true), json('json_schema')]), array('tools', object('', [text('name', true), text('description', true), json('args_json_schema', true), json('output_schema')], true)), output] },
  'embedding.text': { method: 'embedding.text', fields: [array('items', union('', { text: [text('text', true), text('id')], resource: [resource('resource', 'text/*,application/pdf'), text('id')] }), true), num('dimensions', 1), bool('normalize'), text('embedding_space_id'), object('chunking', [select('strategy', ['none', 'auto', 'fixed']), num('max_tokens', 1), num('overlap_tokens')])] },
  'embedding.multimodal': { method: 'embedding.multimodal', fields: [array('items', object('', [text('id', true), text('text'), resource('image', 'image/*', false)], true), true), num('dimensions', 1), bool('normalize')] },
  decision: { method: 'decision.evaluate', fields: [structuredText('state'), array('questions', question, true)] },
  rerank: { method: 'rerank', fields: [text('query', true), array('documents', object('', [text('id', true), text('text'), resource('resource', 'text/*,application/pdf', false), json('metadata')], true), true), num('n', 1), bool('return_documents')] },
  'image.txt2img': { method: 'images.generate', fields: [prompt, text('negative_prompt'), num('n', 1), aspect, select('size', ['256x256', '512x512', '1024x1024', '1536x1024', '1024x1536']), select('quality', ['auto', 'standard', 'hd', 'low', 'medium', 'high']), select('style', ['natural', 'vivid']), num('seed'), output] },
  'image.img2img': { method: 'image.img2img', fields: [array('images', resource(''), true), prompt, num('strength', 0, 1, 0.05), output] },
  'image.inpaint': { method: 'image.inpaint', fields: [resource('image'), resource('mask'), prompt, select('mask_semantics', ['white_area_is_edit_area', 'black_area_is_edit_area', 'alpha_zero_is_edit_area']), output] },
  'image.upscale': { method: 'image.upscale', fields: [resource('image'), num('scale', 1, 16), num('target_width', 1), num('target_height', 1), bool('preserve_faces'), output] },
  'image.bg_remove': { method: 'image.bg_remove', fields: [resource('image'), select('mode', ['auto', 'foreground']), output] },
  'vision.ocr': { method: 'vision.ocr', fields: [resource('document', 'image/*,application/pdf'), select('level', ['page', 'block', 'line', 'word']), checks('language_hints', ['en', 'zh', 'ja', 'ko', 'fr', 'de', 'es']), bool('return_layout'), checks('return_artifacts', ['text', 'markdown', 'json'])] },
  'vision.caption': { method: 'vision.caption', fields: [resource('image'), select('style', ['brief', 'detailed']), language(), num('n', 1)] },
  'vision.detect': { method: 'vision.detect', fields: [resource('image'), array('classes', text('', true)), num('score_threshold', 0, 1, 0.05), object('bbox_spec', [select('format', ['xywh'], true), select('unit', ['px', 'relative'], true)])] },
  'vision.segment': { method: 'vision.segment', fields: [resource('image'), union('prompt', { text: [text('text', true)], point: [{ ...num('x', 0, undefined, 0.01), required: true }, { ...num('y', 0, undefined, 0.01), required: true }, text('label')], box: [bbox] }), select('mask_format', ['rle', 'polygon', 'bitmap_resource']), bool('return_bitmap_mask')] },
  'audio.tts': { method: 'audio.tts', fields: [text('text', true), object('voice', [language(), select('gender', ['female', 'male', 'neutral']), select('style', ['bright', 'upbeat', 'informative', 'firm', 'excitable', 'youthful', 'breezy', 'easy_going', 'breathy', 'clear', 'smooth', 'gravelly', 'soft', 'even', 'mature', 'forward', 'friendly', 'casual', 'gentle', 'lively', 'knowledgeable', 'warm']), text('instructions')], true), num('speed', 0.25, 4, 0.05), output] },
  'audio.asr': { method: 'audio.asr', fields: [resource('audio', 'audio/*'), language(), select('timestamps', ['none', 'segment', 'word']), bool('diarization'), checks('output_formats', ['text', 'json', 'srt', 'vtt'])] },
  'audio.music': { method: 'audio.music', fields: [prompt, duration, bool('instrumental'), text('lyrics'), num('seed'), output] },
  'audio.enhance': { method: 'audio.enhance', fields: [resource('audio', 'audio/*'), select('task', ['denoise', 'dereverb', 'isolate_voice', 'separate_stems'], true), num('strength', 0, 1, 0.05), bool('return_stems')] },
  'video.txt2video': { method: 'video.txt2video', fields: [prompt, duration, aspect, resolution(), bool('generate_audio'), num('seed'), output] },
  'video.img2video': { method: 'video.img2video', fields: [resource('image'), prompt, duration, aspect, resolution()] },
  'video.video2video': { method: 'video.video2video', fields: [resource('video', 'video/*'), prompt, bool('preserve_motion'), object('time_range', [{ ...num('start_seconds', 0, undefined, 0.1), required: true }, { ...num('end_seconds', 0.1, undefined, 0.1), required: true }])] },
  'video.extend': { method: 'video.extend', fields: [resource('video', 'video/*'), prompt, text('continuation_handle'), duration, resolution()] },
  'video.upscale': { method: 'video.upscale', fields: [resource('video', 'video/*'), resolution('target_resolution', true), bool('denoise'), num('sharpen', 0, 1, 0.05), output] },
  'agent.computer_use': { method: 'agent.computer_use', fields: [text('task', true), object('environment', [text('environment_id', true), text('session_id', true), resource('screenshot'), object('viewport', [{ ...num('width', 1), required: true }, { ...num('height', 1), required: true }], true)], true), { ...checks('allowed_actions', ['screenshot', 'left_click', 'right_click', 'type', 'key', 'scroll', 'wait']), required: true }] },
}

export function fieldDefault(field: PlaygroundField): unknown {
  if (field.initial !== undefined) return structuredClone(field.initial)
  if (field.kind === 'object') return fieldsDefault(field.fields ?? [])
  if (field.kind === 'array') return field.required ? Array.from({ length: field.min ?? 1 }, () => fieldDefault(field.item!)) : []
  if (field.kind === 'union') {
    const variant = Object.keys(field.variants!)[0]
    return { [field.discriminator!]: variant, ...fieldsDefault(field.variants![variant]) }
  }
  if (field.kind === 'resource') return { kind: 'url', url: '' }
  if (field.kind === 'boolean') return false
  if (field.kind === 'number') return field.min ?? 0
  if (field.kind === 'select') return field.options?.[0] ?? ''
  if (field.kind === 'checks') return []
  if (field.kind === 'json') return {}
  return ''
}
export function fieldsDefault(fields: PlaygroundField[]): JsonObject {
  return Object.fromEntries(fields.filter((field) => field.required).map((field) => [field.key, fieldDefault(field)]))
}
export function defaultRequest(api: ApiType): JsonObject {
  return { ...fieldsDefault(PLAYGROUND_APIS[api].fields), execution_mode: 'immediate' }
}
export function apiForMethod(method: string): ApiType | undefined {
  return (Object.keys(PLAYGROUND_APIS) as ApiType[]).find((api) => PLAYGROUND_APIS[api].method === method)
}

export function importRequest(source: string, selectedApi: ApiType): { api: ApiType; params: JsonObject; removed: string[] } {
  let value = record(JSON.parse(source))
  let method: string | undefined
  let api = selectedApi
  for (let depth = 0; depth < 8; depth++) {
    if (typeof value.method === 'string') method = value.method
    if (typeof value.api_type === 'string') {
      if (!Object.hasOwn(PLAYGROUND_APIS, value.api_type)) throw new Error('unsupportedApi')
      api = value.api_type as ApiType
    }
    if (typeof value.name === 'string' && value.name.startsWith('AICC ')) method = value.name.slice(5)
    if (typeof value.exact_model === 'string') break
    const nested = ['params', 'input', 'request', 'payload'].find((key) => Object.keys(record(value[key])).length > 0)
    if (!nested) break
    value = record(value[nested])
  }
  if (method) {
    const inferred = apiForMethod(method)
    if (!inferred) throw new Error('unsupportedApi')
    api = inferred
  }
  if (typeof value.exact_model !== 'string' || !value.exact_model.includes('@')) throw new Error('invalidImport')
  const params = structuredClone(value)
  const removed = ['idempotency_key', 'trace_id', 'task_options'].filter((key) => key in params)
  for (const key of [...removed, 'api_type']) delete params[key]
  return { api, params, removed }
}

export interface ValidationIssue { path: string; code: 'required' | 'invalid' | 'range' | 'resource' }
export function validateRequest(api: ApiType, params: JsonObject): ValidationIssue[] {
  const issues: ValidationIssue[] = []
  const issue = (path: string, code: ValidationIssue['code']) => issues.push({ path, code })
  const validate = (field: PlaygroundField, value: unknown, path: string) => {
    if (value === null && field.nullable) return
    if (value === undefined || value === null) { if (field.required) issue(path, 'required'); return }
    if (field.kind === 'text') {
      if (typeof value !== 'string') {
        if (!field.structured || typeof value !== 'object' || Object.keys(value).length === 0) issue(path, 'invalid')
      } else if (field.required && !value.trim()) issue(path, 'required')
    } else if (field.kind === 'number') {
      if (typeof value !== 'number' || !Number.isFinite(value) || (field.step === 1 && !Number.isSafeInteger(value))) issue(path, 'invalid')
      else if (value < (field.min ?? -Infinity) || value > (field.max ?? Infinity)) issue(path, 'range')
    } else if (field.kind === 'boolean' && typeof value !== 'boolean') issue(path, 'invalid')
    else if (field.kind === 'select' && (typeof value !== 'string' || !value)) issue(path, 'invalid')
    else if (field.kind === 'resource') {
      const ref = record(value)
      if (ref.kind === 'url') {
        try { if (!['http:', 'https:'].includes(new URL(String(ref.url)).protocol)) issue(path, 'resource') } catch { issue(path, 'resource') }
      } else if (ref.kind === 'base64') {
        if (typeof ref.mime !== 'string' || !/^\w+[\w.+-]*\/[\w.+-]+$/.test(ref.mime) || typeof ref.data_base64 !== 'string' || !ref.data_base64 || ref.data_base64.length % 4 !== 0 || !/^[A-Za-z0-9+/]*={0,2}$/.test(ref.data_base64)) issue(path, 'resource')
      } else if (ref.kind !== 'named_object' || typeof ref.obj_id !== 'string' || !ref.obj_id.trim()) issue(path, 'resource')
    } else if (field.kind === 'array' || field.kind === 'checks') {
      if (!Array.isArray(value)) { issue(path, 'invalid'); return }
      if (field.required && !value.length) issue(path, 'required')
      else if (value.length < (field.min ?? 0)) issue(path, 'range')
      if (field.item) value.forEach((item, index) => validate(field.item!, item, `${path}[${index}]`))
    } else if (field.kind === 'object' || field.kind === 'union') {
      if (!value || typeof value !== 'object' || Array.isArray(value)) { issue(path, 'invalid'); return }
      const obj = record(value)
      const variant = String(obj[field.discriminator ?? 'type'])
      const fields = field.kind === 'union' ? field.variants && Object.hasOwn(field.variants, variant) ? field.variants[variant] : undefined : field.fields
      if (!fields) { issue(path, 'invalid'); return }
      fields.forEach((child) => validate(child, obj[child.key], `${path}.${child.key}`))
    }
  }
  PLAYGROUND_APIS[api].fields.forEach((field) => validate(field, params[field.key], field.key))
  if (typeof params.exact_model !== 'string' || !params.exact_model.includes('@')) issue('exact_model', 'required')
  if (params.execution_mode !== undefined && !['immediate', 'stream'].includes(String(params.execution_mode))) issue('execution_mode', 'invalid')
  if (api === 'decision' && params.execution_mode === 'stream') issue('execution_mode', 'invalid')
  if (api === 'embedding.multimodal' || api === 'rerank') {
    const key = api === 'rerank' ? 'documents' : 'items'
    if (Array.isArray(params[key])) params[key].forEach((value, index) => {
      const item = record(value)
      if (!item.text && !item.image && !item.resource) issue(`${key}[${index}]`, 'required')
    })
  }
  if (api === 'decision' && Array.isArray(params.questions)) {
    const checkIds = (items: unknown[], path: string) => {
      const ids = items.map((item) => record(item).id)
      if (items.length > 1024 || ids.some((id) => typeof id !== 'string' || !/^[A-Za-z0-9_.-]{1,128}$/.test(id)) || new Set(ids).size !== ids.length) issue(path, 'invalid')
    }
    checkIds(params.questions, 'questions')
    params.questions.forEach((item, index) => {
      const question = record(item)
      if (question.type === 'choice' && Array.isArray(question.options)) checkIds(question.options, `questions[${index}].options`)
      if (question.type === 'score' && Array.isArray(question.levels)) {
        const levels = question.levels.map((level) => JSON.stringify(level))
        if (levels.length > 1024 || new Set(levels).size !== levels.length) issue(`questions[${index}].levels`, 'invalid')
      }
    })
  }
  const range = record(params.time_range)
  if (Object.keys(range).length && !(typeof range.start_seconds === 'number' && typeof range.end_seconds === 'number' && range.end_seconds > range.start_seconds)) issue('time_range', 'range')
  return issues
}

export function taskResponse(task: JsonObject, initial: JsonObject): JsonObject {
  const output = record(record(record(task.result).result).output)
  const terminal = task.phase === 'Terminal'
  return { ...initial, ...record(output.value), ...(output.usage ? { usage: output.usage } : {}), ...(output.cost ? { cost: output.cost } : {}),
    ...(output.artifacts ? { artifacts: output.artifacts } : {}), status: terminal ? task.outcome === 'Succeeded' ? 'succeeded' : task.outcome === 'Canceled' ? 'cancelled' : 'failed' : 'running',
    ...(task.error ? { error: task.error } : {}), task_id: task.task_id ?? initial.task_id }
}

export function resourceUrl(value: unknown): string | undefined {
  const ref = record(value)
  if (ref.kind === 'url' && typeof ref.url === 'string') {
    try { if (['https:', 'http:'].includes(new URL(ref.url).protocol)) return ref.url } catch { return undefined }
  }
  if (ref.kind === 'base64' && typeof ref.mime === 'string' && /^(image\/(png|jpeg|webp|gif)|audio\/[\w.+-]+|video\/[\w.+-]+)$/.test(ref.mime) && typeof ref.data_base64 === 'string') return `data:${ref.mime};base64,${ref.data_base64}`
  return undefined
}
