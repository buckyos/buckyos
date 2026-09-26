import type { ApiType } from './types'
import { record, type JsonObject } from '../datamodel/playground'

export async function mockPlaygroundResponse(api: ApiType, params: JsonObject): Promise<JsonObject> {
  await new Promise((resolve) => setTimeout(resolve, 350))
  const base = { task_id: `mock-${Date.now()}`, status: 'succeeded', usage: { input_tokens: 18, output_tokens: 12, total_tokens: 30 }, cost: { amount: 0, currency: 'USD' }, finish_reason: 'stop', route_trace: { final_model: params.exact_model, attempts: [{ exact_model: params.exact_model, outcome: 'succeeded' }] } }
  if (api === 'llm') return { ...base, message: { role: 'assistant', content: [{ type: 'text', text: 'Mock response: the Playground request was received. Switch off mock mode to test a real Provider.' }] } }
  if (api.startsWith('embedding')) return { ...base, data: (params.items as unknown[]).map((item, index) => ({ index, id: record(item).id, embedding: [0.12, -0.34, 0.56, 0.78], embedding_space_id: 'mock-space' })) }
  if (api === 'decision') return { ...base, answers: (params.questions as JsonObject[]).map((question) => ({ type: question.type, id: question.id, ...(question.type === 'boolean' ? { probability_true: 0.9 } : question.type === 'choice' ? { selected: record((question.options as unknown[])[0]).id, probabilities: Object.fromEntries((question.options as JsonObject[]).map((option, i) => [String(option.id), i === 0 ? 1 : 0])) } : { score: 0, levels: question.levels, probabilities: Object.fromEntries((question.levels as unknown[]).map((_, i) => [String(i), i === 0 ? 1 : 0])) }) })) }
  if (api === 'rerank') return { ...base, results: (params.documents as JsonObject[]).map((document, index) => ({ index, id: document.id, score: 1 / (index + 1), ...(params.return_documents ? { document } : {}) })) }
  if (api === 'vision.caption') return { ...base, captions: [{ text: 'Mock caption', confidence: 0.95 }] }
  if (api === 'vision.detect') return { ...base, detections: [{ label: 'object', score: 0.95, bbox: { format: 'xywh', unit: 'relative', x: 0.1, y: 0.1, width: 0.5, height: 0.5 } }] }
  if (api === 'vision.segment') return { ...base, masks: [{ id: 'mask-1', score: 0.95, mask: { format: 'polygon', points: [[0, 0], [1, 0], [1, 1]] } }] }
  if (api === 'vision.ocr' || api === 'audio.asr') return { ...base, text: 'Mock recognized text.' }
  if (api === 'agent.computer_use') return { ...base, actions: [{ type: 'screenshot' }], requires_next_observation: true }
  const resource = { kind: 'named_object', obj_id: 'mock:generated-resource' }
  if (api.startsWith('image.')) return { ...base, ...(api === 'image.upscale' || api === 'image.bg_remove' ? { image: resource } : { images: [resource] }) }
  return { ...base, [api.startsWith('audio.') ? 'audio' : 'video']: resource }
}
