# i18n 国际化使用指南

## 概述

本项目现在支持自动检测系统语言，并提供了完整的国际化解决方案，支持简体中文和英语。所有UI组件和文本都已支持i18n。

## 功能特性

### 1. 自动语言检测
- 自动检测浏览器/系统的语言设置
- 支持的语言：简体中文 (zh)、英语 (en)
- 如果检测到的语言不支持，默认使用英语

### 2. 语言切换功能
- 支持运行时动态切换语言
- 提供语言切换组件
- 自动更新页面内容

### 3. 完整的UI国际化
- 所有对话框模板已支持i18n
- 所有错误提示和alert文本已国际化
- 支持placeholder、label、value等属性的国际化

## 使用方法

### 1. 在HTML中使用翻译

```html
<!-- 基本文本翻译 -->
<h1 data-i18n="active_title">Active Personal Server</h1>
<p data-i18n="description">This is an example page</p>

<!-- 带参数的翻译 -->
<p data-i18n="greeting" data-i18n-options='{"name": "John"}'>Hello, John!</p>

<!-- HTML内容翻译 -->
<div data-i18n="[html]welcome_message" data-i18n-options='{"user": "Admin"}'>
    Welcome, Admin!
</div>

<!-- 输入框placeholder翻译 -->
<input data-i18n-placeholder="username_placeholder" placeholder="用户名">

<!-- 标签翻译 -->
<md-filled-text-field data-i18n-label="txt_record_label" label="TXT Record"></md-filled-text-field>

<!-- 值翻译 -->
<input data-i18n-value="txt_record_placeholder" value="(请先输入用户名)">
```

### 2. 使用语言切换组件

```html
<!-- 在页面中添加语言切换器 -->
<language-switcher></language-switcher>
```

### 3. 在JavaScript中使用

```typescript
import { changeLanguage, getCurrentLanguage, getSupportedLanguages } from './i18n';

// 切换语言
await changeLanguage('zh'); // 切换到中文
await changeLanguage('en'); // 切换到英文

// 获取当前语言
const currentLang = getCurrentLanguage();

// 获取支持的语言列表
const supportedLangs = getSupportedLanguages();

// 在alert中使用翻译
alert(i18next.t("error_activation_failed") + errorMessage);
```

## 语言文件结构

语言文件位于 `res/` 目录下：

- `res/en.json` - 英语翻译
- `res/zh.json` - 简体中文翻译

### 翻译文件格式

```json
{
    "active_title": "Active Personal Server",
    "greeting": "Hello, {{name}}!",
    "welcome": "Welcome",
    "description": "This is an example page",
    "error_password_mismatch": "The two passwords entered do not match",
    "success_copied": "Content copied to clipboard"
}
```

## 已国际化的组件

### 1. 对话框模板
- `config_gateway_dlg.template` - 网关配置对话框
- `config_zone_id_dlg.template` - 域名配置对话框
- `config_system_dlg.template` - 系统配置对话框
- `final_check_dlg.template` - 最终确认对话框
- `active_result_dlg.template` - 激活结果对话框

### 2. TypeScript文件
- `config_gateway_dlg.ts` - 网关配置逻辑
- `config_zone_id_dlg.ts` - 域名配置逻辑
- `config_system_dlg.ts` - 系统配置逻辑
- `final_check_dlg.ts` - 最终确认逻辑
- `active_result_dlg.ts` - 激活结果逻辑

### 3. 错误提示和Alert
所有错误提示和alert文本都已国际化，包括：
- 密码验证错误
- 用户名验证错误
- 邀请码验证错误
- 域名格式错误
- 激活失败提示
- 复制成功提示

## 添加新的翻译

1. 在 `res/en.json` 中添加英文翻译
2. 在 `res/zh.json` 中添加中文翻译
3. 在HTML中使用相应的 `data-i18n` 属性引用翻译键

## 事件监听

当语言切换时，会触发以下事件：

```typescript
// 监听i18next的语言切换事件
i18next.on('languageChanged', function(lng: string) {
    console.log('Language changed to:', lng);
    // 更新页面内容
});

// 监听自定义语言切换事件
window.addEventListener('languageChanged', function(event: Event) {
    const customEvent = event as CustomEvent;
    console.log('Custom language change event received:', customEvent.detail);
    // 更新页面内容
});
```

## 技术实现

- 使用 `i18next` 作为国际化框架
- 使用 `i18next-http-backend` 加载语言文件
- 自动检测系统语言
- 支持本地存储缓存语言选择
- 提供完整的TypeScript类型支持
- 支持Shadow DOM中的国际化

## 支持的属性

- `data-i18n` - 基本文本翻译
- `data-i18n-placeholder` - 输入框占位符翻译
- `data-i18n-label` - 标签翻译
- `data-i18n-value` - 值翻译
- `data-i18n-options` - 翻译参数

## 注意事项

1. 确保所有翻译键在两种语言文件中都存在
2. 使用 `data-i18n` 属性时，确保翻译键正确
3. 语言切换是异步操作，需要等待完成
4. 页面内容会在语言切换后自动更新
5. Shadow DOM中的元素也会自动更新
6. 所有错误提示和alert都已国际化
