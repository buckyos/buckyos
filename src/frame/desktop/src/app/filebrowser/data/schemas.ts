/**
 * Input models and validation (UI_DATAMODEL.md §3).
 *
 * These schemas define UI constraints — they are not mechanical copies of
 * NFSP request objects. Forms use them as the react-hook-form source of
 * truth; error messages are i18n keys resolved by the rendering layer (see
 * `validationMessage` below for fallbacks).
 */

import { z } from 'zod'

const utf8Length = (value: string) => new TextEncoder().encode(value).length

/**
 * Path-segment names: entries, folders, and Collection groups. The 255 limit
 * is bytes (matches the nfs-server name boundary), not JS character count.
 */
export const entryNameSchema = z
  .string()
  .trim()
  .min(1, 'filebrowser.validation.nameRequired')
  .refine((value) => value !== '.' && value !== '..', {
    message: 'filebrowser.validation.reservedName',
  })
  .refine((value) => !value.includes('/') && !value.includes('\\') && !value.includes('\0'), {
    message: 'filebrowser.validation.invalidNameCharacter',
  })
  .refine((value) => utf8Length(value) <= 255, {
    message: 'filebrowser.validation.nameTooLong',
  })

/** Collection titles are display text — no path-segment restrictions. */
export const collectionTitleSchema = z
  .string()
  .trim()
  .min(1, 'filebrowser.validation.collectionTitleRequired')
  .max(128, 'filebrowser.validation.collectionTitleTooLong')

export const searchInputSchema = z.object({
  query: z
    .string()
    .trim()
    .min(1, 'filebrowser.validation.searchRequired')
    .max(256, 'filebrowser.validation.searchTooLong'),
  scope: z.string().trim().min(1).optional(),
})

export const locationInputSchema = z.object({
  raw: z
    .string()
    .trim()
    .min(1, 'filebrowser.validation.locationRequired')
    .max(2048, 'filebrowser.validation.locationTooLong'),
})

export const listQuerySchema = z
  .object({
    sortKey: z.enum(['manual', 'name', 'size', 'modified', 'kind']),
    sortDir: z.enum(['asc', 'desc']),
    foldersFirst: z.boolean().default(true),
    offset: z.number().int().min(0),
    limit: z.number().int().min(1).max(200).default(200),
  })
  .superRefine((value, context) => {
    if (value.sortKey === 'manual' && value.sortDir !== 'asc') {
      context.addIssue({
        code: 'custom',
        path: ['sortDir'],
        message: 'filebrowser.validation.manualDirectionIgnored',
      })
    }
  })

export const createCollectionInputSchema = z.object({
  title: collectionTitleSchema,
})

export const collectionGroupInputSchema = z.object({
  name: entryNameSchema,
})

export const renameEntryInputSchema = z.object({
  name: entryNameSchema,
})

export const reorderCollectionInputSchema = z.object({
  itemKeys: z.array(z.string().min(1)).min(1),
  toIndex: z.number().int().min(0),
})

/**
 * Upload candidates carry metadata only. The browser `File` object is held
 * out-of-band keyed by `localId` — never serialized into shared state.
 */
export const uploadCandidateSchema = z.object({
  localId: z.string().min(1),
  name: entryNameSchema,
  sizeBytes: z.number().int().min(0),
  mimeType: z.string().max(255).optional(),
  relativePath: z.string().max(2048).optional(),
})

export type SearchInput = z.infer<typeof searchInputSchema>
export type LocationInput = z.infer<typeof locationInputSchema>
export type CreateCollectionInput = z.infer<typeof createCollectionInputSchema>
export type CollectionGroupInput = z.infer<typeof collectionGroupInputSchema>
export type RenameEntryInput = z.infer<typeof renameEntryInputSchema>
export type UploadCandidateInput = z.infer<typeof uploadCandidateSchema>

/**
 * English fallbacks for the validation message keys above — renderers call
 * `t(issue.message, validationFallback[issue.message])`.
 */
export const validationFallback: Record<string, string> = {
  'filebrowser.validation.nameRequired': 'A name is required',
  'filebrowser.validation.reservedName': 'This name is reserved',
  'filebrowser.validation.invalidNameCharacter': 'Names cannot contain /, \\ or NUL',
  'filebrowser.validation.nameTooLong': 'Names are limited to 255 bytes',
  'filebrowser.validation.collectionTitleRequired': 'A collection title is required',
  'filebrowser.validation.collectionTitleTooLong': 'Titles are limited to 128 characters',
  'filebrowser.validation.searchRequired': 'Enter something to search for',
  'filebrowser.validation.searchTooLong': 'Search is limited to 256 characters',
  'filebrowser.validation.locationRequired': 'Enter a location',
  'filebrowser.validation.locationTooLong': 'Locations are limited to 2048 characters',
  'filebrowser.validation.manualDirectionIgnored': 'Manual order has no direction',
}
