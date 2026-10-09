import { LanguageDescription, type LanguageSupport } from '@codemirror/language'
import { languages } from '@codemirror/language-data'
import { contentDescriptor, type TransferableContentRef } from 'buckyos/content'
export { languages }
export async function languageFor(name: string, override?: string, source?: TransferableContentRef): Promise<LanguageSupport | undefined> {
  if (override === 'plain') return undefined
  const description = override ? languages.find(l => l.name === override) : LanguageDescription.matchFilename(languages, name) ??
    (source ? languages.find(l => l.alias.includes(contentDescriptor(source, { name }).mime ?? '')) : undefined)
  return description?.load()
}
