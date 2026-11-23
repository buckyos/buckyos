import { dictionaries } from './dictionaries'
import { currentTextLocale } from './language'

/** Text outside React follows the dictionary this document is displaying. */
export function translateNow(key: string, fallback?: string): string {
  return dictionaries[currentTextLocale()][key] ?? dictionaries.en[key] ?? fallback ?? key
}
