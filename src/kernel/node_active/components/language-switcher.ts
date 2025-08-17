import { changeLanguage, getCurrentLanguage, getSupportedLanguages, getLanguageDisplayName } from '../i18n';

export class LanguageSwitcher extends HTMLElement {
    private selectElement: HTMLSelectElement;

    constructor() {
        super();
        this.selectElement = document.createElement('select');
        this.setupComponent();
    }

    private setupComponent() {
        // 设置样式
        this.style.display = 'inline-block';
        this.style.margin = '10px';
        
        this.selectElement.style.padding = '8px';
        this.selectElement.style.borderRadius = '4px';
        this.selectElement.style.border = '1px solid #ccc';
        this.selectElement.style.fontSize = '14px';
        this.selectElement.style.backgroundColor = '#fff';

        // 添加语言选项
        this.updateLanguageOptions();
        
        // 设置当前选中的语言
        this.selectElement.value = getCurrentLanguage();
        
        // 添加事件监听器
        this.selectElement.addEventListener('change', (event) => {
            const target = event.target as HTMLSelectElement;
            const selectedLanguage = target.value as 'en' | 'zh';
            this.changeLanguage(selectedLanguage);
        });

        // 添加到DOM
        this.appendChild(this.selectElement);
    }

    private updateLanguageOptions() {
        this.selectElement.innerHTML = '';
        
        const supportedLanguages = getSupportedLanguages();
        supportedLanguages.forEach(lang => {
            const option = document.createElement('option');
            option.value = lang;
            option.textContent = getLanguageDisplayName(lang);
            this.selectElement.appendChild(option);
        });
    }

    private async changeLanguage(lang: 'en' | 'zh') {
        try {
            await changeLanguage(lang);
            this.selectElement.value = lang;
            console.log(`Language switched to: ${getLanguageDisplayName(lang)}`);
        } catch (error) {
            console.error('Failed to change language:', error);
        }
    }

    // 公共方法：刷新组件
    public refresh() {
        this.selectElement.value = getCurrentLanguage();
    }
}

// 注册自定义元素
customElements.define('language-switcher', LanguageSwitcher);
