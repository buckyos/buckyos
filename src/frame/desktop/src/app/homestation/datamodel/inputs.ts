import { z } from 'zod'

export const audienceSchema = z.discriminatedUnion('kind', [
  z.object({ kind: z.literal('public') }),
  z.object({ kind: z.literal('followers') }),
  z.object({ kind: z.literal('friends') }),
  z.object({ kind: z.literal('group'), groupId: z.string().min(1, 'homestation.validation.groupRequired') }),
  z.object({
    kind: z.literal('dids'),
    dids: z.array(z.string().trim().regex(/^did:[a-z0-9]+:.+/i, 'homestation.validation.didFormat')).min(1, 'homestation.validation.didRequired').max(20),
  }),
])

export const attachmentSchema = z.object({
  id: z.string().min(1),
  kind: z.enum(['image', 'video', 'audio']),
  name: z.string().min(1).max(200),
  status: z.enum(['uploading', 'uploaded', 'failed']),
  object: z.string().optional(),
})

export const linkCardSchema = z.object({
  url: z.string().trim().regex(/^https?:\/\/[^\s/]+\.[^\s]+$/i, 'homestation.validation.url'),
  title: z.string().trim().min(1, 'homestation.validation.linkTitle').max(120),
  summary: z.string().trim().max(280),
})

export const publishInputSchema = z
  .object({
    text: z.string().trim().max(2000, 'homestation.validation.textTooLong'),
    attachments: z.array(attachmentSchema).max(9, 'homestation.validation.tooManyAttachments'),
    link: linkCardSchema.nullable(),
    audience: audienceSchema,
    /** Also list the post in the zone feed (§4.6); sent only with a public audience. */
    zoneFeed: z.boolean().optional(),
  })
  .superRefine((value, ctx) => {
    if (!value.text && value.attachments.length === 0 && !value.link) {
      ctx.addIssue({ code: 'custom', path: ['text'], message: 'homestation.validation.empty' })
    }
    if (value.attachments.some(attachment => attachment.status !== 'uploaded' || !attachment.object)) {
      ctx.addIssue({ code: 'custom', path: ['attachments'], message: 'homestation.validation.uploadPending' })
    }
    if (value.attachments.filter(attachment => attachment.kind === 'video').length > 1 || value.attachments.filter(attachment => attachment.kind === 'audio').length > 1) {
      ctx.addIssue({ code: 'custom', path: ['attachments'], message: 'homestation.validation.oneVideoOrAudio' })
    }
  })

export type AudienceInput = z.infer<typeof audienceSchema>
export type AttachmentInput = z.infer<typeof attachmentSchema>
export type LinkCardInput = z.infer<typeof linkCardSchema>
export type PublishInput = z.infer<typeof publishInputSchema>

export const commentInputSchema = z.object({
  text: z.string().trim().min(1, 'homestation.validation.commentEmpty').max(1000, 'homestation.validation.textTooLong'),
})

export type CommentInput = z.infer<typeof commentInputSchema>

export const sourceInputSchema = z
  .object({
    kind: z.enum(['follow', 'url', 'natural']),
    text: z.string().trim().min(1, 'homestation.validation.sourceEmpty').max(300),
  })
  .superRefine((value, ctx) => {
    if (value.kind === 'url' && !/^https?:\/\/[^\s/]+\.[^\s]+$/i.test(value.text)) {
      ctx.addIssue({ code: 'custom', path: ['text'], message: 'homestation.validation.url' })
    }
    if (value.kind === 'natural' && value.text.length < 4) {
      ctx.addIssue({ code: 'custom', path: ['text'], message: 'homestation.validation.intentTooShort' })
    }
    if (value.kind === 'follow' && value.text.length < 2) {
      ctx.addIssue({ code: 'custom', path: ['text'], message: 'homestation.validation.followTooShort' })
    }
  })

export type SourceInput = z.infer<typeof sourceInputSchema>

export const filterRuleInputSchema = z.object({
  enabled: z.boolean(),
  conditions: z.array(z.enum(['ai_full', 'ai_assisted', 'low_quality'])).min(1, 'homestation.validation.conditionRequired'),
  acceptInferred: z.boolean(),
  minConfidence: z.number().min(0.5).max(0.99),
  unknown: z.enum(['show', 'hide']),
})

export type FilterRuleInput = z.infer<typeof filterRuleInputSchema>
