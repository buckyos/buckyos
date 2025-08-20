import Handlebars from 'handlebars';
import i18next from '../i18n';

// 使用MutationObserver的双向绑定
export class MutationHandlebarsComponent extends HTMLElement {
    private data: any;
    private template: HandlebarsTemplateDelegate;
    private shadow: ShadowRoot;
    private observer: MutationObserver;

    constructor(templateContent: string, initialData: any) {
        super();
        this.data = { ...initialData };
        this.template = Handlebars.compile(templateContent);
        this.shadow = this.attachShadow({ mode: 'open' });
        this.observer = new MutationObserver(this.handleMutation.bind(this));
        this.render();
        this.setupObserver();
    }

    private render() {
        const params = {
            ...this.data,
            invite_code_placeholder: i18next.t("invite_code_placeholder"),
            custom_sn_placeholder: i18next.t("custom_sn_placeholder"),
            use_buckyos_sn: i18next.t("use_buckyos_sn"),
            direct_connect_label: i18next.t("direct_connect_label")
        };
        
        this.shadow.innerHTML = this.template(params);
        this.setupEventListeners();
    }

    private setupObserver() {
        this.observer.observe(this.shadow, {
            childList: true,
            subtree: true,
            attributes: true,
            attributeFilter: ['value', 'checked']
        });
    }

    private handleMutation(mutations: MutationRecord[]) {
        mutations.forEach(mutation => {
            if (mutation.type === 'attributes') {
                const target = mutation.target as HTMLElement;
                const key = target.getAttribute('data-bind');
                if (key) {
                    if (target instanceof HTMLInputElement) {
                        if (target.type === 'checkbox') {
                            this.data[key] = target.checked;
                        } else {
                            this.data[key] = target.value;
                        }
                    }
                }
            }
        });
    }

    private setupEventListeners() {
        // 监听输入框变化
        const inputs = this.shadow.querySelectorAll('input, textarea, select');
        inputs.forEach(input => {
            input.addEventListener('input', (e) => {
                const target = e.target as HTMLInputElement;
                const key = target.getAttribute('data-bind');
                if (key) {
                    this.data[key] = target.value;
                    console.log(`Data updated: ${key} = ${target.value}`);
                }
            });
        });

        // 监听复选框变化
        const checkboxes = this.shadow.querySelectorAll('input[type="checkbox"]');
        checkboxes.forEach(checkbox => {
            checkbox.addEventListener('change', (e) => {
                const target = e.target as HTMLInputElement;
                const key = target.getAttribute('data-bind');
                if (key) {
                    this.data[key] = target.checked;
                    console.log(`Data updated: ${key} = ${target.checked}`);
                }
            });
        });
    }

    // 更新数据并重新渲染
    updateData(newData: any) {
        Object.assign(this.data, newData);
        this.render();
    }

    // 获取当前数据
    getData() {
        return { ...this.data };
    }

    // 设置单个数据项
    setData(key: string, value: any) {
        this.data[key] = value;
        // 更新对应的DOM元素
        const element = this.shadow.querySelector(`[data-bind="${key}"]`) as HTMLInputElement;
        if (element) {
            if (element.type === 'checkbox') {
                element.checked = value;
            } else {
                element.value = value;
            }
        }
    }

    disconnectedCallback() {
        this.observer.disconnect();
    }
}
