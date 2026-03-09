import i18next from 'i18next';
import HttpBackend from 'i18next-http-backend';
import { initReactI18next } from 'react-i18next';

const LANGUAGE_STORAGE_KEY = 'node_active.language';

export const LANGUAGE_OPTIONS = [
  {
    code: 'zh',
    label: '中文',
    flag: '🇨🇳',
    resource: 'zh',
    aliases: ['zh', 'zh-cn', 'zh-sg'],
  },
  {
    code: 'zh-TW',
    label: '繁体中文',
    flag: '🇹🇼',
    resource: 'zh-TW',
    aliases: ['zh-tw', 'zh-hk', 'zh-mo', 'zh-hant'],
  },
  {
    code: 'en',
    label: 'English',
    flag: '🇺🇸',
    resource: 'en',
    aliases: ['en', 'en-us', 'en-gb', 'en-ca', 'en-au'],
  },
  {
    code: 'es',
    label: 'Español',
    flag: '🇪🇸',
    resource: 'es',
    aliases: ['es', 'es-es', 'es-mx', 'es-419'],
  },
  {
    code: 'fr',
    label: 'Français',
    flag: '🇫🇷',
    resource: 'fr',
    aliases: ['fr', 'fr-fr', 'fr-ca', 'fr-be', 'fr-ch'],
  },
  {
    code: 'de',
    label: 'Deutsch',
    flag: '🇩🇪',
    resource: 'de',
    aliases: ['de', 'de-de', 'de-at', 'de-ch'],
  },
  {
    code: 'ko',
    label: '한국어',
    flag: '🇰🇷',
    resource: 'ko',
    aliases: ['ko', 'ko-kr'],
  },
  {
    code: 'ja',
    label: '日本語',
    flag: '🇯🇵',
    resource: 'ja',
    aliases: ['ja', 'ja-jp'],
  },
  {
    code: 'ru',
    label: 'Русский',
    flag: '🇷🇺',
    resource: 'ru',
    aliases: ['ru', 'ru-ru'],
  },
] as const;

export type SupportedLanguage = (typeof LANGUAGE_OPTIONS)[number]['code'];

type LanguageOption = (typeof LANGUAGE_OPTIONS)[number];

function getLanguageOption(lang?: string | null): LanguageOption | undefined {
  if (!lang) {
    return undefined;
  }

  const normalized = lang.toLowerCase();
  return LANGUAGE_OPTIONS.find(
    (option) =>
      option.code.toLowerCase() === normalized ||
      option.aliases.some((alias) => alias === normalized) ||
      normalized.startsWith(`${option.code.toLowerCase()}-`),
  );
}

function normalizeLanguage(lang?: string | null): SupportedLanguage {
  return getLanguageOption(lang)?.code ?? 'en';
}

function getLanguageResource(lang?: string | null): string {
  return getLanguageOption(lang)?.resource ?? 'en';
}

function syncDocumentLanguage(lang: SupportedLanguage) {
  if (typeof document !== 'undefined') {
    document.documentElement.lang = lang;
  }
}

function getStoredLanguage(): SupportedLanguage | null {
  try {
    const stored = localStorage.getItem(LANGUAGE_STORAGE_KEY);
    return stored ? normalizeLanguage(stored) : null;
  } catch (error) {
    console.warn('Error reading stored language:', error);
    return null;
  }
}

// 检测系统语言
function detectSystemLanguage(): SupportedLanguage {
  try {
    const candidates = [navigator.language, ...(navigator.languages ?? [])].filter(Boolean);
    console.log('Detected system languages:', candidates);

    for (const systemLang of candidates) {
      const language = getLanguageOption(systemLang);
      if (language) {
        console.log('Using language:', language.code);
        return language.code;
      }
    }

    console.log('Using default language: en');
    return 'en';
  } catch (error) {
    console.warn('Error detecting system language:', error);
    return 'en';
  }
}

const initialLanguage = getStoredLanguage() ?? detectSystemLanguage();

i18next
  .use(HttpBackend)
  .use(initReactI18next)
  .init({
    lng: initialLanguage, // 使用检测到的系统语言
    fallbackLng: 'en', // 降级语言
    supportedLngs: LANGUAGE_OPTIONS.map((option) => option.code), // 支持的语言列表
    load: 'currentOnly',
    backend: {
      loadPath: (languages: readonly string[] | string) => {
        const lang = Array.isArray(languages) ? languages[0] : languages;
        return `${getLanguageResource(lang)}.json`;
      },
      crossDomain: false,
      withCredentials: false
    },
    ns: ['common'], 
    defaultNS: 'common',
    detection: {
      // 语言检测选项
      order: ['navigator', 'htmlTag', 'path', 'subdomain'],
      caches: ['localStorage']
    },
    debug: false, // 生产环境可以设置为false
    interpolation: {
      escapeValue: false // React已经处理了XSS
    },
    react: {
      useSuspense: false
    }
  }).then(() => {
    const language = getCurrentLanguage();
    syncDocumentLanguage(language);
    console.log("i18n initialized with language:", language);
  }).catch((error) => {
    console.error("i18n initialization failed:", error);
  });

// 等待i18n初始化的工具函数
export function waitForI18n(): Promise<void> {
  return new Promise((resolve) => {
    if (i18next.isInitialized) {
      resolve();
    } else {
      i18next.on('initialized', resolve);
    }
  });
}

// 语言切换函数
export async function changeLanguage(lang: SupportedLanguage): Promise<void> {
  const normalized = normalizeLanguage(lang);

  try {
    await i18next.changeLanguage(normalized);
    try {
      localStorage.setItem(LANGUAGE_STORAGE_KEY, normalized);
    } catch (error) {
      console.warn('Error storing language:', error);
    }
    syncDocumentLanguage(normalized);
    console.log('Language changed to:', normalized);

    // 触发自定义事件，通知其他组件更新
    window.dispatchEvent(new CustomEvent('languageChanged', { detail: { language: normalized } }));
  } catch (error) {
    console.error('Failed to change language:', error);
  }
}

// 获取当前语言
export function getCurrentLanguage(): SupportedLanguage {
  return normalizeLanguage(i18next.resolvedLanguage || i18next.language || getStoredLanguage() || detectSystemLanguage());
}

// 获取支持的语言列表
export function getSupportedLanguages(): SupportedLanguage[] {
  return LANGUAGE_OPTIONS.map((option) => option.code);
}

export function getLanguageOptions(): readonly LanguageOption[] {
  return LANGUAGE_OPTIONS;
}

// 获取语言显示名称
export function getLanguageDisplayName(lang: string): string {
  return getLanguageOption(lang)?.label || lang;
}

export function getLanguageFlag(lang: string): string {
  return getLanguageOption(lang)?.flag || '🌐';
}

export default i18next;
