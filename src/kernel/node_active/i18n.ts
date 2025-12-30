import i18next from 'i18next';
import HttpBackend from 'i18next-http-backend';
import { initReactI18next } from 'react-i18next';

// 检测系统语言
function detectSystemLanguage(): string {
  try {
    const systemLang = navigator.language || navigator.languages?.[0] || 'en';
    console.log('Detected system language:', systemLang);
    
    // 支持的语言列表
    const supportedLanguages = ['en', 'zh'];
    
    // 检查完整语言代码（如 zh-CN, zh-TW, zh-HK 等）
    if (systemLang.toLowerCase().startsWith('zh')) {
      console.log('Using Chinese language');
      return 'zh';
    }
    
    // 检查简化语言代码
    const langCode = systemLang.split('-')[0].toLowerCase();
    if (supportedLanguages.includes(langCode)) {
      console.log('Using language:', langCode);
      return langCode;
    }
    
    // 默认返回英语
    console.log('Using default language: en');
    return 'en';
  } catch (error) {
    console.warn('Error detecting system language:', error);
    return 'en';
  }
}

i18next
  .use(HttpBackend)
  .use(initReactI18next)
  .init({
    lng: detectSystemLanguage(), // 使用检测到的系统语言
    fallbackLng: 'en', // 降级语言
    supportedLngs: ['en', 'zh'], // 支持的语言列表
    backend: {
      loadPath: '{{lng}}.json', // 修正路径指向res目录
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
    console.log("i18n initialized with language:", i18next.language);
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
export async function changeLanguage(lang: 'en' | 'zh'): Promise<void> {
  try {
    await i18next.changeLanguage(lang);
    console.log('Language changed to:', lang);
    
    // 触发自定义事件，通知其他组件更新
    window.dispatchEvent(new CustomEvent('languageChanged', { detail: { language: lang } }));
  } catch (error) {
    console.error('Failed to change language:', error);
  }
}

// 获取当前语言
export function getCurrentLanguage(): string {
  const lng = i18next.language || detectSystemLanguage();
  return lng.split('-')[0];
}

// 获取支持的语言列表
export function getSupportedLanguages(): string[] {
  return ['en', 'zh'];
}

// 获取语言显示名称
export function getLanguageDisplayName(lang: string): string {
  const displayNames: Record<string, string> = {
    'en': 'English',
    'zh': '简体中文'
  };
  return displayNames[lang] || lang;
}

export default i18next;
