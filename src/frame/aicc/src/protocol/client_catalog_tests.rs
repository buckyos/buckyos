//! Keeps `doc/aicc/protocol_clients.md` in step with the registered protocol
//! clients. Adding, removing or reshaping a client fails this test until the
//! catalog below and the document are updated with it.

use super::CodecRegistry;
use std::collections::BTreeSet;

/// (client, layer, platform). The layer is the review decision recorded in
/// the protocol-client TODO §3.4; the registry cannot derive it.
const CLIENTS: &[(&str, &str, &str)] = &[
    ("openai-responses", "A 标准，官方口径", "OpenAI"),
    ("openai-chat-completions", "A 标准，官方口径", "OpenAI"),
    ("openai-embeddings", "A 标准，官方口径", "OpenAI"),
    ("openai-images", "A 标准，官方口径", "OpenAI"),
    ("openai-audio", "A 标准，官方口径", "OpenAI"),
    ("openai-videos", "A 标准，官方口径", "OpenAI"),
    ("claude-messages", "A 标准，官方口径", "Anthropic"),
    ("gemini-generate-content", "A 标准，官方口径", "Google"),
    ("rerank", "A 标准，官方口径", "Cohere / Jina 口径"),
    (
        "legacy-openai-chat-completions",
        "A 标准，生态口径",
        "OpenAI 兼容生态",
    ),
    (
        "legacy-claude-messages",
        "A 标准，生态口径",
        "Anthropic 兼容生态",
    ),
    ("gemini-interactions", "B 官方独有", "Google"),
    ("bailian-responses", "方言", "阿里云百炼"),
    ("openrouter-responses", "方言", "OpenRouter"),
    ("fal-queue", "C 原生", "fal"),
    ("minimax-media", "C 原生", "MiniMax"),
    ("doubao-media", "C 原生", "火山方舟"),
    ("doubao-speech", "C 原生", "豆包语音"),
    ("bailian-media", "C 原生", "阿里云百炼"),
    ("glm-video", "C 原生", "智谱"),
    ("openrouter-native", "C 原生", "OpenRouter"),
    ("typesafe-systemone", "C 原生", "TypeSafe"),
];

fn render(codecs: &CodecRegistry, custom: &BTreeSet<String>) -> Vec<String> {
    let mut lines = vec![
        "| 客户端 | 层 | 平台 | 协议族 | 基础客户端 | 对自定义开放 | operation |".to_owned(),
        "| --- | --- | --- | --- | --- | --- | --- |".to_owned(),
    ];
    for (id, layer, platform) in CLIENTS {
        let adapter = codecs
            .adapter(id)
            .unwrap_or_else(|| panic!("catalog client {id} is not registered"));
        let operations = adapter
            .operations
            .keys()
            .map(|operation| format!("`{operation}`"))
            .collect::<Vec<_>>()
            .join("<br>");
        lines.push(format!(
            "| `{id}` | {layer} | {platform} | `{}` | {} | {} | {operations} |",
            adapter.protocol_family_id,
            adapter
                .base_adapter_id
                .as_deref()
                .map_or("—".to_owned(), |base| format!("`{base}`")),
            if custom.contains(*id) { "是" } else { "否" },
        ));
    }
    lines
}

#[test]
fn protocol_client_catalog_document_matches_the_registry() {
    let catalog = crate::settings::MetadataSources {
        builtin: crate::settings::load_builtin_metadata().unwrap(),
        ..Default::default()
    }
    .build_snapshot(
        crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
        &crate::catalog::CatalogBuildOptions::default(),
    )
    .unwrap();
    let codecs = crate::provider::builtin_provider_registry(&catalog)
        .unwrap()
        .codecs();
    let listed = CLIENTS
        .iter()
        .map(|(id, _, _)| *id)
        .collect::<BTreeSet<_>>();
    assert_eq!(listed.len(), CLIENTS.len(), "a client is listed twice");
    let registered = codecs
        .adapters()
        .map(|adapter| adapter.protocol_adapter_id.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        listed, registered,
        "the catalog must list every registered client"
    );
    let rendered = render(&codecs, &catalog.custom_provider_adapter_ids());
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../doc/aicc/protocol_clients.md"
    );
    let document = std::fs::read_to_string(path).unwrap();
    let (head, rest) = document
        .split_once("<!-- BEGIN GENERATED CLIENTS -->")
        .unwrap();
    let (section, tail) = rest.split_once("<!-- END GENERATED CLIENTS -->").unwrap();
    if std::env::var("AICC_UPDATE_CLIENT_CATALOG_DOC").is_ok_and(|value| value == "1") {
        std::fs::write(
            path,
            format!(
                "{head}<!-- BEGIN GENERATED CLIENTS -->\n{}\n<!-- END GENERATED CLIENTS -->{tail}",
                rendered.join("\n")
            ),
        )
        .unwrap();
        return;
    }
    let documented = section
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(
        rendered, documented,
        "run with AICC_UPDATE_CLIENT_CATALOG_DOC=1"
    );
    for (id, layer, _) in CLIENTS {
        if layer.starts_with("方言") {
            assert!(
                tail.contains(&format!("### `{id}`")),
                "dialect {id} needs a residual-difference section"
            );
        }
    }
}
