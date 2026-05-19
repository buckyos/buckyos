use std::collections::BTreeMap;
use std::path::Path;

const DEFAULT_LANGUAGE: &str = "en";

#[derive(Debug, Clone, Copy)]
struct LanguageOption {
    code: &'static str,
    aliases: &'static [&'static str],
}

const LANGUAGE_OPTIONS: &[LanguageOption] = &[
    LanguageOption {
        code: "zh",
        aliases: &["zh", "zh-cn", "zh-sg"],
    },
    LanguageOption {
        code: "zh-TW",
        aliases: &["zh-tw", "zh-hk", "zh-mo", "zh-hant"],
    },
    LanguageOption {
        code: "en",
        aliases: &["en", "en-us", "en-gb", "en-ca", "en-au"],
    },
    LanguageOption {
        code: "es",
        aliases: &["es", "es-es", "es-mx", "es-419"],
    },
    LanguageOption {
        code: "fr",
        aliases: &["fr", "fr-fr", "fr-ca", "fr-be", "fr-ch"],
    },
    LanguageOption {
        code: "de",
        aliases: &["de", "de-de", "de-at", "de-ch"],
    },
    LanguageOption {
        code: "ko",
        aliases: &["ko", "ko-kr"],
    },
    LanguageOption {
        code: "ja",
        aliases: &["ja", "ja-jp"],
    },
    LanguageOption {
        code: "ru",
        aliases: &["ru", "ru-ru"],
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentI18n {
    language: String,
    messages: BTreeMap<String, String>,
}

impl Default for AgentI18n {
    fn default() -> Self {
        Self {
            language: DEFAULT_LANGUAGE.to_string(),
            messages: default_messages(DEFAULT_LANGUAGE),
        }
    }
}

impl AgentI18n {
    pub fn load(root: &Path, language: &str) -> Self {
        let language = normalize_language(language).to_string();
        let i18n_dir = root.join("i18n");
        let mut messages = default_messages(DEFAULT_LANGUAGE);
        merge_messages(
            &mut messages,
            load_language_file(&i18n_dir, DEFAULT_LANGUAGE),
        );
        if language != DEFAULT_LANGUAGE {
            merge_messages(&mut messages, default_messages(&language));
            merge_messages(&mut messages, load_language_file(&i18n_dir, &language));
        }
        Self { language, messages }
    }

    pub fn language(&self) -> &str {
        self.language.as_str()
    }

    pub fn render(&self, key: &str, args: &[(&str, String)]) -> String {
        let mut out = self
            .messages
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string());
        for (name, value) in args {
            out = out.replace(&format!("{{{name}}}"), value);
        }
        out
    }
}

fn normalize_language(language: &str) -> &'static str {
    let normalized = language.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return DEFAULT_LANGUAGE;
    }

    for option in LANGUAGE_OPTIONS {
        if option.code.eq_ignore_ascii_case(&normalized)
            || option.aliases.iter().any(|alias| *alias == normalized)
        {
            return option.code;
        }
    }

    for option in LANGUAGE_OPTIONS {
        let prefix = format!("{}-", option.code.to_ascii_lowercase());
        if normalized.starts_with(&prefix) {
            return option.code;
        }
    }

    DEFAULT_LANGUAGE
}

fn merge_messages(base: &mut BTreeMap<String, String>, next: BTreeMap<String, String>) {
    for (key, value) in next {
        base.insert(key, value);
    }
}

fn load_language_file(i18n_dir: &Path, language: &str) -> BTreeMap<String, String> {
    let path = i18n_dir.join(format!("{language}.toml"));
    let Ok(content) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(value) = content.parse::<toml::Value>() else {
        return BTreeMap::new();
    };
    let mut out = BTreeMap::new();
    flatten_toml("", &value, &mut out);
    out
}

fn flatten_toml(prefix: &str, value: &toml::Value, out: &mut BTreeMap<String, String>) {
    match value {
        toml::Value::String(s) if !prefix.is_empty() => {
            out.insert(prefix.to_string(), s.clone());
        }
        toml::Value::Table(table) => {
            for (key, child) in table {
                let next = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_toml(&next, child, out);
            }
        }
        _ => {}
    }
}

fn default_messages(language: &str) -> BTreeMap<String, String> {
    let entries = match language {
        "zh" => [
            ("status.llm_started", "正在思考 ({model})"),
            ("status.llm_finished", "思考完成"),
            ("status.llm_failed", "思考失败"),
            ("status.llm_error", "模型调用出错: {error}"),
            ("status.tool_planned", "准备使用工具: {tool}"),
            ("status.tool_finished", "工具 {tool} {result}"),
            ("status.tool_result_done", "完成"),
            ("status.tool_result_failed", "失败"),
            ("status.tool_failed", "工具 {tool} 失败: {message}"),
            ("status.parse_error", "解析结果失败: {error}"),
            (
                "status.context_rewritten",
                "已压缩历史: {from_messages} → {to_messages}",
            ),
            ("status.self_report_set", "已更新自述 ({chars} 字符)"),
            (
                "status.message_sent",
                "已发送消息给 {target} ({chars} 字符)",
            ),
        ],
        "zh-TW" => [
            ("status.llm_started", "正在思考 ({model})"),
            ("status.llm_finished", "思考完成"),
            ("status.llm_failed", "思考失敗"),
            ("status.llm_error", "模型呼叫出錯: {error}"),
            ("status.tool_planned", "準備使用工具: {tool}"),
            ("status.tool_finished", "工具 {tool} {result}"),
            ("status.tool_result_done", "完成"),
            ("status.tool_result_failed", "失敗"),
            ("status.tool_failed", "工具 {tool} 失敗: {message}"),
            ("status.parse_error", "解析結果失敗: {error}"),
            (
                "status.context_rewritten",
                "已壓縮歷史: {from_messages} → {to_messages}",
            ),
            ("status.self_report_set", "已更新自述 ({chars} 字元)"),
            (
                "status.message_sent",
                "已傳送訊息給 {target} ({chars} 字元)",
            ),
        ],
        "es" => [
            ("status.llm_started", "LLM pensando ({model})"),
            ("status.llm_finished", "LLM finalizado"),
            ("status.llm_failed", "LLM falló"),
            ("status.llm_error", "Error del LLM: {error}"),
            ("status.tool_planned", "herramienta: {tool}"),
            ("status.tool_finished", "herramienta {tool} {result}"),
            ("status.tool_result_done", "completada"),
            ("status.tool_result_failed", "falló"),
            ("status.tool_failed", "herramienta {tool} falló: {message}"),
            ("status.parse_error", "error de análisis: {error}"),
            (
                "status.context_rewritten",
                "historial comprimido: {from_messages} → {to_messages}",
            ),
            (
                "status.self_report_set",
                "autoinforme actualizado ({chars} caracteres)",
            ),
            (
                "status.message_sent",
                "mensaje enviado a {target} ({chars} caracteres)",
            ),
        ],
        "fr" => [
            ("status.llm_started", "LLM en réflexion ({model})"),
            ("status.llm_finished", "LLM terminé"),
            ("status.llm_failed", "LLM échoué"),
            ("status.llm_error", "Erreur LLM: {error}"),
            ("status.tool_planned", "outil: {tool}"),
            ("status.tool_finished", "outil {tool} {result}"),
            ("status.tool_result_done", "terminé"),
            ("status.tool_result_failed", "échoué"),
            ("status.tool_failed", "outil {tool} échoué: {message}"),
            ("status.parse_error", "erreur d'analyse: {error}"),
            (
                "status.context_rewritten",
                "historique compressé: {from_messages} → {to_messages}",
            ),
            (
                "status.self_report_set",
                "auto-rapport mis à jour ({chars} caractères)",
            ),
            (
                "status.message_sent",
                "message envoyé à {target} ({chars} caractères)",
            ),
        ],
        "de" => [
            ("status.llm_started", "LLM denkt nach ({model})"),
            ("status.llm_finished", "LLM abgeschlossen"),
            ("status.llm_failed", "LLM fehlgeschlagen"),
            ("status.llm_error", "LLM-Fehler: {error}"),
            ("status.tool_planned", "Tool: {tool}"),
            ("status.tool_finished", "Tool {tool} {result}"),
            ("status.tool_result_done", "abgeschlossen"),
            ("status.tool_result_failed", "fehlgeschlagen"),
            (
                "status.tool_failed",
                "Tool {tool} fehlgeschlagen: {message}",
            ),
            ("status.parse_error", "Parse-Fehler: {error}"),
            (
                "status.context_rewritten",
                "Verlauf komprimiert: {from_messages} → {to_messages}",
            ),
            (
                "status.self_report_set",
                "Selbstbericht aktualisiert ({chars} Zeichen)",
            ),
            (
                "status.message_sent",
                "Nachricht an {target} gesendet ({chars} Zeichen)",
            ),
        ],
        "ko" => [
            ("status.llm_started", "LLM 생각 중 ({model})"),
            ("status.llm_finished", "LLM 완료"),
            ("status.llm_failed", "LLM 실패"),
            ("status.llm_error", "LLM 오류: {error}"),
            ("status.tool_planned", "도구: {tool}"),
            ("status.tool_finished", "도구 {tool} {result}"),
            ("status.tool_result_done", "완료"),
            ("status.tool_result_failed", "실패"),
            ("status.tool_failed", "도구 {tool} 실패: {message}"),
            ("status.parse_error", "파싱 오류: {error}"),
            (
                "status.context_rewritten",
                "기록 압축됨: {from_messages} → {to_messages}",
            ),
            ("status.self_report_set", "자기 보고 업데이트됨 ({chars}자)"),
            (
                "status.message_sent",
                "{target}에게 메시지 전송됨 ({chars}자)",
            ),
        ],
        "ja" => [
            ("status.llm_started", "LLM 思考中 ({model})"),
            ("status.llm_finished", "LLM 完了"),
            ("status.llm_failed", "LLM 失敗"),
            ("status.llm_error", "LLM エラー: {error}"),
            ("status.tool_planned", "ツール: {tool}"),
            ("status.tool_finished", "ツール {tool} {result}"),
            ("status.tool_result_done", "完了"),
            ("status.tool_result_failed", "失敗"),
            ("status.tool_failed", "ツール {tool} 失敗: {message}"),
            ("status.parse_error", "解析エラー: {error}"),
            (
                "status.context_rewritten",
                "履歴を圧縮: {from_messages} → {to_messages}",
            ),
            (
                "status.self_report_set",
                "自己レポート更新済み ({chars} 文字)",
            ),
            (
                "status.message_sent",
                "{target} にメッセージ送信済み ({chars} 文字)",
            ),
        ],
        "ru" => [
            ("status.llm_started", "LLM думает ({model})"),
            ("status.llm_finished", "LLM завершил работу"),
            ("status.llm_failed", "Сбой LLM"),
            ("status.llm_error", "Ошибка LLM: {error}"),
            ("status.tool_planned", "инструмент: {tool}"),
            ("status.tool_finished", "инструмент {tool} {result}"),
            ("status.tool_result_done", "готово"),
            ("status.tool_result_failed", "сбой"),
            (
                "status.tool_failed",
                "инструмент {tool} завершился ошибкой: {message}",
            ),
            ("status.parse_error", "ошибка разбора: {error}"),
            (
                "status.context_rewritten",
                "история сжата: {from_messages} → {to_messages}",
            ),
            (
                "status.self_report_set",
                "самоотчет обновлен ({chars} симв.)",
            ),
            (
                "status.message_sent",
                "сообщение отправлено {target} ({chars} симв.)",
            ),
        ],
        _ => [
            ("status.llm_started", "LLM thinking ({model})"),
            ("status.llm_finished", "LLM finished"),
            ("status.llm_failed", "LLM failed"),
            ("status.llm_error", "LLM error: {error}"),
            ("status.tool_planned", "tool: {tool}"),
            ("status.tool_finished", "tool {tool} {result}"),
            ("status.tool_result_done", "done"),
            ("status.tool_result_failed", "failed"),
            ("status.tool_failed", "tool {tool} failed: {message}"),
            ("status.parse_error", "parse error: {error}"),
            (
                "status.context_rewritten",
                "compressed history: {from_messages} → {to_messages}",
            ),
            ("status.self_report_set", "self-report set ({chars} chars)"),
            ("status.message_sent", "message → {target} ({chars} chars)"),
        ],
    };

    entries
        .into_iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn falls_back_to_default_messages() {
        let dir = tempdir().unwrap();
        let i18n = AgentI18n::load(dir.path(), "zh-CN");
        assert_eq!(i18n.language(), "zh");
        assert_eq!(
            i18n.render("status.tool_planned", &[("tool", "read".to_string())]),
            "准备使用工具: read"
        );
    }

    #[test]
    fn loads_nested_toml_messages() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("i18n")).unwrap();
        std::fs::write(
            dir.path().join("i18n/zh.toml"),
            "[status]\ntool_planned = \"工具: {tool}\"\n",
        )
        .unwrap();
        let i18n = AgentI18n::load(dir.path(), "zh-CN");
        assert_eq!(
            i18n.render("status.tool_planned", &[("tool", "read".to_string())]),
            "工具: read"
        );
    }

    #[test]
    fn normalizes_system_language_aliases() {
        assert_eq!(normalize_language("en-US"), "en");
        assert_eq!(normalize_language("zh-HK"), "zh-TW");
        assert_eq!(normalize_language("fr-FR"), "fr");
        assert_eq!(normalize_language("unknown"), "en");
    }
}
