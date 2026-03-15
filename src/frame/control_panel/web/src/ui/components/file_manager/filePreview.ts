export type FilePreviewKind = 'image' | 'pdf' | 'text' | 'docx' | 'office' | 'audio' | 'video' | 'unknown'
export type TextPreviewMode = 'plain' | 'markdown' | 'json' | 'csv' | 'code'

const IMAGE_EXTENSIONS = new Set(['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'svg'])
const TEXT_DOCUMENT_EXTENSIONS = new Set([
  'txt',
  'md',
  'markdown',
  'json',
  'yaml',
  'yml',
  'toml',
  'ini',
  'conf',
  'log',
  'csv',
  'xml',
])
const MARKDOWN_EXTENSIONS = new Set(['md', 'markdown'])
const JSON_EXTENSIONS = new Set(['json'])
const CSV_EXTENSIONS = new Set(['csv'])
const CODE_TEXT_EXTENSIONS = new Set([
  'js',
  'jsx',
  'ts',
  'tsx',
  'mjs',
  'cjs',
  'rs',
  'py',
  'go',
  'java',
  'kt',
  'swift',
  'c',
  'h',
  'cpp',
  'hpp',
  'sh',
  'bash',
  'zsh',
  'sql',
  'rb',
  'php',
  'scala',
  'lua',
  'dart',
  'vue',
  'svelte',
])
const DOC_EXTENSION = 'doc'
const DOCX_EXTENSION = 'docx'
const OFFICE_DOCUMENT_EXTENSIONS = new Set(['doc', 'xls', 'xlsx', 'ppt', 'pptx', 'odt', 'ods', 'odp'])
const AUDIO_EXTENSIONS = new Set(['mp3', 'wav', 'ogg', 'm4a', 'flac', 'aac'])
const VIDEO_EXTENSIONS = new Set(['mp4', 'webm', 'ogv', 'mov', 'm4v', 'mkv'])

export const getFileExtension = (name: string) => {
  const dot = name.lastIndexOf('.')
  if (dot < 0 || dot === name.length - 1) {
    return ''
  }
  return name.slice(dot + 1).toLowerCase()
}

export const isDocFileName = (name: string) => getFileExtension(name) === DOC_EXTENSION

export const isDocxFileName = (name: string) => getFileExtension(name) === DOCX_EXTENSION

export const getTextPreviewMode = (name: string): TextPreviewMode => {
  const ext = getFileExtension(name)
  if (MARKDOWN_EXTENSIONS.has(ext)) {
    return 'markdown'
  }
  if (JSON_EXTENSIONS.has(ext)) {
    return 'json'
  }
  if (CSV_EXTENSIONS.has(ext)) {
    return 'csv'
  }
  if (CODE_TEXT_EXTENSIONS.has(ext)) {
    return 'code'
  }
  return 'plain'
}

export const getFilePreviewKind = (entry: { name: string }): FilePreviewKind => {
  const ext = getFileExtension(entry.name)
  if (IMAGE_EXTENSIONS.has(ext)) {
    return 'image'
  }
  if (ext === 'pdf') {
    return 'pdf'
  }
  if (TEXT_DOCUMENT_EXTENSIONS.has(ext) || CODE_TEXT_EXTENSIONS.has(ext)) {
    return 'text'
  }
  if (ext === DOCX_EXTENSION) {
    return 'docx'
  }
  if (OFFICE_DOCUMENT_EXTENSIONS.has(ext)) {
    return 'office'
  }
  if (AUDIO_EXTENSIONS.has(ext)) {
    return 'audio'
  }
  if (VIDEO_EXTENSIONS.has(ext)) {
    return 'video'
  }
  return 'unknown'
}
