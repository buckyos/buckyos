/** The Desktop's languages: the product's set (src/kernel/node_active/i18n.ts: Simplified and Traditional Chinese,
 * English, Spanish, French, German, Korean, Japanese, Russian) plus Arabic, which the Desktop shell had before. */
export const supportedLocales = [
  'en',
  'zh-CN',
  'zh-TW',
  'ja',
  'ko',
  'fr',
  'de',
  'es',
  'ru',
  'ar',
] as const

export type SupportedLocale = (typeof supportedLocales)[number]

/** Browser and system tags that name a Chinese script or region rather than the plain language. */
const CHINESE_TAGS: Record<string, SupportedLocale> = {
  zh: 'zh-CN', 'zh-cn': 'zh-CN', 'zh-sg': 'zh-CN', 'zh-hans': 'zh-CN', 'zh-my': 'zh-CN',
  'zh-tw': 'zh-TW', 'zh-hk': 'zh-TW', 'zh-mo': 'zh-TW', 'zh-hant': 'zh-TW',
}

/** The Desktop language for a BCP 47 tag (`ja-JP`, `zh-Hant-HK`, `pt-BR`…), or null when there is none. */
export function matchSupportedLocale(tag: string | null | undefined): SupportedLocale | null {
  if (!tag) return null
  const lower = tag.trim().replace(/_/g, '-').toLowerCase()
  const exact = supportedLocales.find((locale) => locale.toLowerCase() === lower)
  if (exact) return exact
  const parts = lower.split('-')
  if (parts[0] === 'zh') {
    // the script first (zh-Hant-HK, zh-Hans-TW…), then the region, then Simplified
    return CHINESE_TAGS[`zh-${parts[1] ?? ''}`] ?? CHINESE_TAGS[`zh-${parts[2] ?? ''}`] ?? 'zh-CN'
  }
  return supportedLocales.find((locale) => locale === parts[0]) ?? null
}

export function browserLocale(): SupportedLocale {
  return (navigator.languages ?? [navigator.language]).map(matchSupportedLocale).find(locale => locale !== null)
    ?? matchSupportedLocale(navigator.language) ?? 'en'
}
