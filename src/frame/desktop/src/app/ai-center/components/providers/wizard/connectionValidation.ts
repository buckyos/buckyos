import { z } from 'zod'
import type { WizardDraft } from '../../../../../api/aicc_mgr'

export const wizardDraftSchema = z.object({
  provider_instance_name: z.string().trim().min(1).max(64).optional(),
  provider_profile_id: z.enum(['sn', 'openai', 'claude', 'gemini', 'fal', 'openrouter', 'minimax', 'kimi', 'glm', 'deepseek', 'doubao', 'qwen', 'custom']).nullable(),
  display_name: z.string().trim().max(80),
  base_url: z.string().trim(),
  protocol_family_id: z.string().nullable(),
  protocol_adapter_id: z.string().optional(),
  region: z.string().optional(),
  workspace: z.string().optional(),
  account: z.string().optional(),
  auth_mode: z.enum(['api_key', 'dynamic_login']),
  api_key: z.string(),
  auto_sync_models: z.boolean(),
}).superRefine((draft, context) => {
  if (!draft.provider_profile_id) return
  if (!draft.base_url) context.addIssue({ code: 'custom', path: ['base_url'], message: 'Base URL is required' })
  if (draft.provider_profile_id === 'custom' && !draft.protocol_family_id) {
    context.addIssue({ code: 'custom', path: ['protocol_family_id'], message: 'Protocol family is required' })
  }
  if (draft.auth_mode === 'api_key' && !draft.api_key.trim()) {
    context.addIssue({ code: 'custom', path: ['api_key'], message: 'API key is required' })
  }
})

export function isConnectionValid(draft: WizardDraft): boolean {
  return wizardDraftSchema.safeParse(draft).success
}
