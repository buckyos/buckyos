# AICC 统一匹配规则

状态：Beta 2.2 目标规范

本文定义 AICC metadata、Provider Rules、请求规则、价格规则和发布 track 共用的匹配语义。统一的是匹配实现和行为，不要求调用者为简单场景填写复杂对象。

## 1. 设计原则

1. 字符串是首选表示。绝大多数模型名、Provider 名或 API type 匹配直接写一个完整字符串或 wildcard。
2. 只有需要同时约束多个维度时才展开为对象。
3. 同一匹配实现负责完整字符串匹配、wildcard、数组任选、否定、存在性和有序范围；各业务 schema 不再发明自己的 predicate DSL。
4. `MatchRule` 只判断是否命中，不携带 `exclude`、价格、operation、参数改写等动作。
5. 不支持脚本、任意表达式、递归布尔树或配置侧正则表达式。

## 2. 表示形式

`MatchRule` 是以下两种形式之一：

```text
MatchRule = string | MatchRuleObject
```

### 2.1 字符串简写

```json
"gpt-5-*"
```

字符串作用于所在业务字段声明的主维度。例如 Model Driver pattern 的主维度是 `origin_model_id`，Provider model rule 默认是 `provider_model_id`，发布 track 的主维度是 `client_version`。

- 不含 wildcard 的字符串表示完整值精确匹配；
- `*` 匹配任意长度字符，`?` 匹配单个字符；
- 匹配默认区分大小写并覆盖完整字段，不做子串搜索；
- 字面量 `*`、`?` 和 `\` 分别写成 `\*`、`\?` 和 `\\`。

简单规则不得为了形式统一被改写成对象。例如应写：

```json
"openai/gpt-5*"
```

而不是：

```json
{
  "provider_model_id": {
    "glob": "openai/gpt-5*"
  }
}
```

### 2.2 多维对象

当一条规则需要联合多个维度时，使用对象；对象各字段之间固定为 AND：

```json
{
  "provider_model_id": "openai/gpt-5*",
  "api_type": ["llm", "vision.*"],
  "update_channel": "stable"
}
```

字段值支持：

- scalar：精确匹配；字符串 scalar 同时支持 wildcard；
- array：任一元素匹配即成立，即 OR；空数组永不匹配；
- `{ "not": value }`：该值不匹配；
- `{ "exists": true|false }`：判断维度是否存在；
- `{ "min": value, "max": value, "include_min": bool, "include_max": bool }`：只用于 schema 明确声明为可排序的版本或数值维度，边界默认包含。

例如：

```json
{
  "client_version": {
    "min": "2.2.0",
    "max": "2.3.0",
    "include_max": false
  },
  "update_channel": ["stable", "beta"],
  "rollout_group": "cn-*"
}
```

对象中未知维度、同一维度使用不兼容 operator、范围用于不可排序字段，均属于配置错误，不能按不匹配静默忽略。

## 3. 业务字段绑定

每个使用 `MatchRule` 的 schema 必须声明：

- 字符串简写对应的主维度；
- 允许在对象中出现的维度及其类型；
- 哪些维度允许范围比较；
- 规则列表的顺序和冲突处理。

首版统一用于：

| 场景 | 主维度 | 常见扩展维度 |
| --- | --- | --- |
| Model Driver model pattern / variant / version rule | `origin_model_id` | `family`、`tier`、`stability`、`api_type` |
| Provider model rule | `provider_model_id` | `origin_model_id`、`model_driver_id`、`variant`、`api_type` |
| Request rule / pricing rule | 无，必须使用扁平对象 | normalized option path、`api_type`、`operation` |
| routing policy 的 Provider/model 范围 | 当前列表对应的 Provider 或 exact model | `api_type`、`logical_path` |
| metadata 发布 track | `client_version` | `update_channel`、`rollout_group` |

请求参数维度使用 normalized option 的 JSON Pointer 作为 key，例如：

```json
{
  "/quality": "high",
  "/size": ["1536x1024", "1024x1536"]
}
```

## 4. 顺序与动作

`MatchRule` 不决定规则优先级。包含它的业务规则列表负责定义顺序，默认使用有序列表的第一条命中规则；需要累计应用的 request rules 必须由对应 schema 明确声明。

动作保留在外层：

```json
{
  "match": "*/*latest*",
  "exclude": true
}
```

```json
{
  "when": {
    "/quality": "high",
    "/size": ["1536x1024", "1024x1536"]
  },
  "amount": 0.167
}
```

## 5. 不属于 MatchRule 的能力

需要从字符串中提取命名片段的 `origin_mappings[].extract` 属于解析/转换规则，可以由内置代码使用正则捕获组，但它不是 `MatchRule`，也不能把该能力开放成所有匹配字段都可使用的通用正则。普通 catalog 和用户配置始终优先使用 wildcard `MatchRule`。

## 6. 实现约束

- 所有业务 schema 反序列化后归一化为同一个内部 `CompiledMatchRule`，不能分别实现 wildcard 或条件判断。
- catalog、settings 或发布 index 加载时完成校验和编译；推理热路径只执行已编译规则。
- trace 至少记录命中的规则 ID/数组位置和实际参与的维度，不记录凭据或敏感 option 值。
- 相同输入和规则必须得到确定性结果；不得依赖 map 遍历顺序。
- 实现不得为此引入新的 regex、表达式语言或规则引擎依赖；如确需新增依赖，必须单独评审。

## 7. 最小验收

- 字符串精确匹配、`*`、`?` 和转义行为一致；
- 字符串简写与等价的单维对象结果一致；
- 对象维度为 AND、数组值为 OR；
- `not`、`exists` 和允许范围的边界行为确定；
- 未知维度和非法 operator 在加载时被拒绝；
- Model Driver、Provider Rules、request/pricing rules 和发布 track 共用同一组 matcher contract tests；
- 简单 Provider 配置示例只使用字符串 wildcard，不要求用户理解对象形式。
