/**
 * Presentation FileKind classification (UI_DATAMODEL.md §2.1): a backend file
 * node is classified by its name/MIME; unknown types render as `other`.
 * Shared by the mock transfer executor and the NFSP adapter.
 */

import type { FileKind } from '../types'

const KIND_BY_EXT: Record<string, FileKind> = {
  jpg: 'image',
  jpeg: 'image',
  png: 'image',
  gif: 'image',
  webp: 'image',
  svg: 'image',
  heic: 'image',
  mp4: 'video',
  mov: 'video',
  mkv: 'video',
  webm: 'video',
  mp3: 'audio',
  flac: 'audio',
  wav: 'audio',
  m4a: 'audio',
  zip: 'archive',
  tar: 'archive',
  gz: 'archive',
  '7z': 'archive',
  rar: 'archive',
  ts: 'code',
  tsx: 'code',
  js: 'code',
  jsx: 'code',
  rs: 'code',
  py: 'code',
  go: 'code',
  css: 'code',
  html: 'code',
  json: 'code',
  yaml: 'code',
  yml: 'code',
  toml: 'code',
  sh: 'code',
  pdf: 'document',
  doc: 'document',
  docx: 'document',
  xls: 'document',
  xlsx: 'document',
  ppt: 'document',
  pptx: 'document',
  md: 'document',
  txt: 'document',
}

const KIND_BY_MIME_PREFIX: Record<string, FileKind> = {
  image: 'image',
  video: 'video',
  audio: 'audio',
  text: 'document',
}

/** Classify a file by extension, with an optional MIME-type fallback. */
export function classifyFileKind(name: string, mimeType?: string): FileKind {
  const ext = name.split('.').pop()?.toLowerCase() ?? ''
  const byExt = KIND_BY_EXT[ext]
  if (byExt) return byExt
  const prefix = mimeType?.split('/')[0]
  return (prefix && KIND_BY_MIME_PREFIX[prefix]) || 'other'
}
