import { matchSupportedLocale, type SupportedLocale } from '../../../shared/locale.ts'
import ar from './locales/ar.json' with { type: 'json' }
import de from './locales/de.json' with { type: 'json' }
import en from './locales/en.json' with { type: 'json' }
import es from './locales/es.json' with { type: 'json' }
import fr from './locales/fr.json' with { type: 'json' }
import ja from './locales/ja.json' with { type: 'json' }
import ko from './locales/ko.json' with { type: 'json' }
import ru from './locales/ru.json' with { type: 'json' }
import zhCN from './locales/zh-CN.json' with { type: 'json' }
import zhTW from './locales/zh-TW.json' with { type: 'json' }

const dictionaries = { en, ar, de, es, fr, ja, ko, ru, 'zh-CN': zhCN, 'zh-TW': zhTW } satisfies Record<SupportedLocale, typeof en>

export function translate(locale: string, key: string): string {
  const dictionary: Record<string, string> = dictionaries[matchSupportedLocale(locale) ?? 'en']
  return Object.hasOwn(dictionary, key) ? dictionary[key] : key
}

export function editorPhrases(locale: string): Record<string, string> {
  return Object.fromEntries(Object.entries(dictionaries[matchSupportedLocale(locale) ?? 'en'])
    .filter(([key]) => key.startsWith('cm.'))
    .map(([key, text]) => [key.slice(3), text]))
}
