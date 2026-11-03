//! LLMContext -> CYFS MessageHub message bridge (egress).
//!
//! This module is the explicit protocol boundary between two different
//! object models:
//!
//! - `ndn_lib::MsgObject` is the CYFS / MessageHub object. It is immutable,
//!   addressable, and optimized for cross-zone delivery. Human text lives in
//!   `MsgContent.content`; non-structured attachments are `MsgContent.refs`
//!   pointing at CYFS data objects; machine-readable control payloads live in
//!   `MsgContent.machine`.
//! - `buckyos_api::AiMessage` is the LLMContext waist message. It is a
//!   provider-neutral inference input/output carrier with ordered
//!   `AiContent` blocks such as text, image, document, tool calls and
//!   provider state.
//!
//! [`ai_message_to_msg_object_with_base_validated_async`] turns a reply into
//! a `MsgObject`, keeping these rules deliberately narrow:
//!
//! 1. Text stays text. Multiple text blocks from an LLM response are joined
//!    with blank lines.
//! 2. LLM non-text content is mapped to MessageHub attachments whenever it
//!    can be represented by `MsgContent.refs`. `ResourceRef::NamedObject`
//!    becomes a `RefItem::DataObj`. Provider-only sources such as URL or
//!    base64 cannot be losslessly represented as a CYFS ref, so they are
//!    serialized into a small textual marker:
//!
//!    `<attachment kind="image" source="url" url="https://..." mime="image/png"/>`
//!
//!    The same marker is accepted inside `AiContent::Text`, allowing an LLM
//!    that can only emit text to request an outgoing attachment by object id
//!    or local path:
//!
//!    `<attachment kind="document" obj_id="file:0123abcd" title="report.pdf"/>`
//!
//! Inbound messages are rendered by `runner::input_view`; it only borrows
//! [`attachment_kind`] / [`attachment_mime`] from here.
//!
//! This file should remain free of OpenDAN session policy. It only translates
//! protocol shapes; routing, authorization and reply policy belong to the
//! caller ([`AttachmentValidator`], [`LocalFileResolver`]).

use std::collections::BTreeMap;

use buckyos_api::{AiContent, AiMessage, AiRole, ResourceRef};
use ndn_lib::{
    MachineContent, MsgContent, MsgContentFormat, MsgObject, ObjId, RefItem, RefRole, RefTarget,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use agent_tool::xml_util::{parse_attrs, xml_escape};

const PROVIDER_MSG_MACHINE: &str = "buckyos.msg.machine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachmentTag {
    pub kind: String,
    pub obj_id: Option<String>,
    pub url: Option<String>,
    pub path: Option<String>,
    pub mime: Option<String>,
    pub title: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MsgParserError {
    #[error("invalid attachment obj_id `{value}`: {message}")]
    InvalidObjId { value: String, message: String },
}

/// Result of validating a single LLM-emitted `<attachment>` reference.
/// `Ok(())` lowers it to a `MsgContent.refs` entry; `Err(reason)` keeps it
/// as inert text so the message isn't silently corrupted.
pub type AttachmentValidation = std::result::Result<(), String>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MsgEgressOptions {
    pub preserve_attachment_tag_in_egress: bool,
}

/// Out-of-band ACL / path policy injected at the egress conversion site.
/// `msg_parser` itself is policy-free — opendan supplies a real
/// implementation (workspace whitelist, ACL lookups, …); the default
/// methods approve everything.
pub trait AttachmentValidator: Send + Sync {
    fn validate_obj_id(&self, _obj_id: &ObjId) -> AttachmentValidation {
        Ok(())
    }
    fn validate_path(&self, _path: &str) -> AttachmentValidation {
        Ok(())
    }
    fn validate_url(&self, _url: &str) -> AttachmentValidation {
        Ok(())
    }
}

/// Materializes an LLM-emitted local file path into a CYFS `ObjId` so the
/// attachment can travel as a content-addressed `RefItem::DataObj` (the
/// same lane used by uploads coming from outside). The expected
/// implementation registers the file with NamedStore in LocalLink mode so
/// no bytes are copied — identical content yields identical ObjIds and
/// dedupes automatically.
///
/// `Err(reason)` lets the caller render an `<!-- attachment rejected: … -->`
/// note instead of silently dropping the attachment.
#[async_trait::async_trait]
pub trait LocalFileResolver: Send + Sync {
    async fn resolve(&self, path: &str) -> std::result::Result<ObjId, String>;
}

/// Convert an `AiMessage` into a `MsgObject`, preserving the caller-provided
/// envelope fields in `base` and replacing only `base.content`. Every
/// `<attachment>` reference produced by the LLM (obj_id / path / url) is
/// filtered through `validator`; rejected attachments are left inline as text
/// so the model's intent is preserved verbatim. Routes
/// validated `<attachment>BODY</attachment>` local paths through `resolver`
/// (typically NamedStore LocalLink registration) so they leave as
/// content-addressed `RefItem::DataObj` and are uploadable by tunnels
/// (Telegram, …) the same way externally-sourced attachments are.
pub async fn ai_message_to_msg_object_with_base_validated_async(
    message: &AiMessage,
    base: MsgObject,
    validator: &dyn AttachmentValidator,
    options: MsgEgressOptions,
    resolver: &dyn LocalFileResolver,
) -> Result<MsgObject, MsgParserError> {
    let mut text_parts: Vec<String> = Vec::new();
    let mut refs: Vec<RefItem> = Vec::new();
    let mut machine_payloads: Vec<Value> = Vec::new();

    for block in &message.content {
        match block {
            AiContent::Text { text } => {
                collect_text_and_attachment_tags_async(
                    text,
                    &mut text_parts,
                    &mut refs,
                    validator,
                    options,
                    resolver,
                )
                .await?;
            }
            AiContent::Image { source } => {
                collect_resource_ref("image", source, None, &mut text_parts, &mut refs, validator);
            }
            AiContent::Document { source, title } => {
                collect_resource_ref(
                    "document",
                    source,
                    title.as_deref(),
                    &mut text_parts,
                    &mut refs,
                    validator,
                );
            }
            AiContent::ProviderState {
                provider, value, ..
            } if provider == PROVIDER_MSG_MACHINE => {
                machine_payloads.push(value.clone());
            }
            AiContent::ToolUse { .. }
            | AiContent::ToolResult { .. }
            | AiContent::Thinking { .. }
            | AiContent::ProviderState { .. } => {}
        }
    }

    Ok(assemble_msg_object(
        base,
        message.role,
        text_parts,
        refs,
        machine_payloads,
    ))
}

fn assemble_msg_object(
    mut base: MsgObject,
    role: AiRole,
    text_parts: Vec<String>,
    refs: Vec<RefItem>,
    machine_payloads: Vec<Value>,
) -> MsgObject {
    let content = text_parts
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    base.content = MsgContent {
        format: if content.is_empty() {
            None
        } else {
            Some(MsgContentFormat::TextPlain)
        },
        content,
        refs,
        machine: machine_payloads_to_machine(machine_payloads),
        ..MsgContent::default()
    };
    base.meta.insert(
        "llm_role".to_string(),
        Value::String(role.as_str().to_string()),
    );
    base
}

/// `image | audio | video | document | file` of a data object reference,
/// inferred from its label, URI hint and the message format (in that order).
pub fn attachment_kind(
    msg_format: Option<&MsgContentFormat>,
    label: Option<&str>,
    uri_hint: Option<&str>,
) -> &'static str {
    label
        .and_then(attachment_kind_from_hint)
        .or_else(|| uri_hint.and_then(attachment_kind_from_hint))
        .or_else(|| msg_format.and_then(attachment_kind_from_format))
        .unwrap_or("file")
}

fn attachment_kind_from_format(format: &MsgContentFormat) -> Option<&'static str> {
    match format {
        MsgContentFormat::ImagePng
        | MsgContentFormat::ImageJpeg
        | MsgContentFormat::ImageGif
        | MsgContentFormat::ImageWebp
        | MsgContentFormat::ImageSvg
        | MsgContentFormat::ImageBmp => Some("image"),
        MsgContentFormat::VideoMp4
        | MsgContentFormat::VideoWebm
        | MsgContentFormat::VideoOgg
        | MsgContentFormat::VideoQuicktime
        | MsgContentFormat::VideoAvi => Some("video"),
        MsgContentFormat::AudioMpeg
        | MsgContentFormat::AudioWav
        | MsgContentFormat::AudioOgg
        | MsgContentFormat::AudioWebm
        | MsgContentFormat::AudioAac
        | MsgContentFormat::AudioFlac => Some("audio"),
        MsgContentFormat::TextPlain
        | MsgContentFormat::TextMarkdown
        | MsgContentFormat::TextHtml
        | MsgContentFormat::TextCss
        | MsgContentFormat::TextXml
        | MsgContentFormat::ApplicationJson
        | MsgContentFormat::ApplicationXml
        | MsgContentFormat::ApplicationPdf => Some("document"),
        MsgContentFormat::ApplicationZip | MsgContentFormat::ApplicationOctetStream => Some("file"),
        MsgContentFormat::Unknown(value) => attachment_kind_from_hint(value),
    }
}

fn attachment_kind_from_hint(value: &str) -> Option<&'static str> {
    let value = value.trim().to_ascii_lowercase();
    if value.starts_with("image/")
        || [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp"]
            .iter()
            .any(|suffix| value.ends_with(suffix))
    {
        Some("image")
    } else if value.starts_with("audio/")
        || [".mp3", ".wav", ".ogg", ".aac", ".flac", ".m4a"]
            .iter()
            .any(|suffix| value.ends_with(suffix))
    {
        Some("audio")
    } else if value.starts_with("video/")
        || [".mp4", ".webm", ".mov", ".avi", ".mkv"]
            .iter()
            .any(|suffix| value.ends_with(suffix))
    {
        Some("video")
    } else if value.starts_with("text/")
        || matches!(
            value.as_str(),
            "application/pdf" | "application/json" | "application/xml"
        )
        || [".pdf", ".md", ".txt", ".json", ".xml", ".html"]
            .iter()
            .any(|suffix| value.ends_with(suffix))
    {
        Some("document")
    } else if value.starts_with("application/")
        || [".zip", ".tar", ".gz", ".bin"]
            .iter()
            .any(|suffix| value.ends_with(suffix))
    {
        Some("file")
    } else {
        None
    }
}

/// MIME type of a data object reference, when it can be inferred.
pub fn attachment_mime(
    msg_format: Option<&MsgContentFormat>,
    label: Option<&str>,
    uri_hint: Option<&str>,
) -> Option<String> {
    label
        .and_then(attachment_mime_from_hint)
        .or_else(|| uri_hint.and_then(attachment_mime_from_hint))
        .or_else(|| msg_format.and_then(msg_content_format_mime))
}

fn attachment_mime_from_hint(value: &str) -> Option<String> {
    let value = value.trim().to_ascii_lowercase();
    let media_type = value.split(';').next().unwrap_or_default().trim();
    if let Some((major, subtype)) = media_type.split_once('/') {
        if matches!(major, "image" | "audio" | "video" | "text" | "application")
            && !subtype.is_empty()
            && !subtype.contains(['/', '\\'])
        {
            return Some(media_type.to_string());
        }
    }

    let path = value.split(['?', '#']).next().unwrap_or_default();
    [
        (".png", "image/png"),
        (".jpg", "image/jpeg"),
        (".jpeg", "image/jpeg"),
        (".gif", "image/gif"),
        (".webp", "image/webp"),
        (".svg", "image/svg+xml"),
        (".bmp", "image/bmp"),
        (".mp3", "audio/mpeg"),
        (".wav", "audio/wav"),
        (".ogg", "audio/ogg"),
        (".aac", "audio/aac"),
        (".flac", "audio/flac"),
        (".m4a", "audio/mp4"),
        (".mp4", "video/mp4"),
        (".webm", "video/webm"),
        (".mov", "video/quicktime"),
        (".avi", "video/x-msvideo"),
        (".mkv", "video/x-matroska"),
        (".pdf", "application/pdf"),
        (".md", "text/markdown"),
        (".txt", "text/plain"),
        (".json", "application/json"),
        (".xml", "application/xml"),
        (".html", "text/html"),
        (".zip", "application/zip"),
    ]
    .into_iter()
    .find_map(|(suffix, mime)| path.ends_with(suffix).then(|| mime.to_string()))
}

fn msg_content_format_mime(format: &MsgContentFormat) -> Option<String> {
    serde_json::to_value(format)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
}

fn collect_resource_ref(
    kind: &str,
    source: &ResourceRef,
    title: Option<&str>,
    text_parts: &mut Vec<String>,
    refs: &mut Vec<RefItem>,
    validator: &dyn AttachmentValidator,
) {
    match source {
        ResourceRef::NamedObject { obj_id } => {
            if let Err(reason) = validator.validate_obj_id(obj_id) {
                text_parts.push(render_attachment_rejection(
                    &AttachmentTag {
                        kind: kind.to_string(),
                        obj_id: Some(obj_id.to_string()),
                        url: None,
                        path: None,
                        mime: None,
                        title: title.map(|s| s.to_string()),
                        label: None,
                    },
                    &reason,
                ));
                return;
            }
            refs.push(RefItem {
                role: RefRole::Input,
                target: RefTarget::DataObj {
                    obj_id: obj_id.clone(),
                    uri_hint: None,
                },
                label: title.map(|s| s.to_string()),
            });
        }
        ResourceRef::Url { url, mime_hint } => {
            if let Err(reason) = validator.validate_url(url) {
                text_parts.push(render_attachment_rejection(
                    &AttachmentTag {
                        kind: kind.to_string(),
                        obj_id: None,
                        url: Some(url.clone()),
                        path: None,
                        mime: mime_hint.clone(),
                        title: title.map(|s| s.to_string()),
                        label: None,
                    },
                    &reason,
                ));
                return;
            }
            text_parts.push(render_attachment_tag(&AttachmentTag {
                kind: kind.to_string(),
                obj_id: None,
                url: Some(url.clone()),
                path: None,
                mime: mime_hint.clone(),
                title: title.map(|s| s.to_string()),
                label: None,
            }));
        }
        ResourceRef::Base64 { mime, data_base64 } => {
            text_parts.push(format!(
                "<attachment kind=\"{}\" source=\"base64\" mime=\"{}\" data_base64=\"{}\"{} />",
                xml_escape(kind),
                xml_escape(mime),
                xml_escape(data_base64),
                title
                    .map(|s| format!(" title=\"{}\"", xml_escape(s)))
                    .unwrap_or_default()
            ));
        }
    }
}

/// Text blocks with `<attachment>` markers. Pending local-file
/// references are handed to `resolver` and materialized into
/// `RefItem::DataObj` entries — typically the resolver registers the file
/// with NamedStore in LocalLink mode so identical content yields identical
/// ObjIds without copying bytes. Resolver failures degrade to rejection
/// text so the LLM's intent stays visible and audit-logged.
async fn collect_text_and_attachment_tags_async(
    text: &str,
    text_parts: &mut Vec<String>,
    refs: &mut Vec<RefItem>,
    validator: &dyn AttachmentValidator,
    options: MsgEgressOptions,
    resolver: &dyn LocalFileResolver,
) -> Result<(), MsgParserError> {
    let mut items = collect_egress_line_items(text, validator)?;
    resolve_pending_attachments_async(&mut items, resolver).await;
    finalize_egress_items_into(items, text_parts, refs, options);
    Ok(())
}

enum EgressLineItem {
    Text(String),
    Tag {
        tag: AttachmentTag,
        lowering: AttachmentLowering,
    },
}

fn collect_egress_line_items(
    text: &str,
    validator: &dyn AttachmentValidator,
) -> Result<Vec<EgressLineItem>, MsgParserError> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(tag) = parse_attachment_tag(line.trim()) {
            let lowering = attachment_tag_to_ref_item(&tag, validator)?;
            out.push(EgressLineItem::Tag { tag, lowering });
        } else {
            out.push(EgressLineItem::Text(line.to_string()));
        }
    }
    Ok(out)
}

async fn resolve_pending_attachments_async(
    items: &mut [EgressLineItem],
    resolver: &dyn LocalFileResolver,
) {
    for item in items.iter_mut() {
        if let EgressLineItem::Tag { tag, lowering } = item {
            let path = match lowering {
                AttachmentLowering::PendingPath(p) => p.clone(),
                _ => continue,
            };
            *lowering = match resolver.resolve(&path).await {
                Ok(obj_id) => AttachmentLowering::Ref(RefItem {
                    role: RefRole::Input,
                    target: RefTarget::DataObj {
                        obj_id,
                        uri_hint: None,
                    },
                    label: tag.title.clone().or_else(|| tag.label.clone()),
                }),
                Err(reason) => {
                    AttachmentLowering::Rejected(format!("local file resolve failed: {reason}"))
                }
            };
        }
    }
}

fn finalize_egress_items_into(
    items: Vec<EgressLineItem>,
    text_parts: &mut Vec<String>,
    refs: &mut Vec<RefItem>,
    options: MsgEgressOptions,
) {
    let mut ordinary = Vec::new();
    for item in items {
        match item {
            EgressLineItem::Text(s) => ordinary.push(s),
            EgressLineItem::Tag { tag, lowering } => match lowering {
                AttachmentLowering::Ref(item) => {
                    refs.push(item);
                    if options.preserve_attachment_tag_in_egress {
                        ordinary.push(render_attachment_tag(&tag));
                    }
                }
                AttachmentLowering::Keep | AttachmentLowering::PendingPath(_) => {
                    ordinary.push(render_attachment_tag(&tag));
                }
                AttachmentLowering::Rejected(reason) => {
                    ordinary.push(render_attachment_rejection(&tag, &reason));
                }
            },
        }
    }
    let joined = ordinary.join("\n").trim().to_string();
    if !joined.is_empty() {
        text_parts.push(joined);
    }
}

enum AttachmentLowering {
    /// Lowered to a `MsgContent.refs` entry.
    Ref(RefItem),
    /// Nothing structured to lower — keep the tag as text.
    Keep,
    /// Validator rejected the reference; keep an annotated rejection text.
    Rejected(String),
    /// Local-path attachment validated but not yet materialized. The async
    /// egress path hands this to a `LocalFileResolver` (typically NamedStore
    /// LocalLink registration) to obtain the content-addressed ObjId that
    /// promotes it to `Ref`.
    PendingPath(String),
}

fn attachment_tag_to_ref_item(
    tag: &AttachmentTag,
    validator: &dyn AttachmentValidator,
) -> Result<AttachmentLowering, MsgParserError> {
    if let Some(raw_path) = tag.path.as_ref().filter(|s| !s.trim().is_empty()) {
        // Validator owns the path-policy decision (workspace fence / `..`
        // traversal / symlink-escape). The actual file → ObjId
        // materialization is deferred to a `LocalFileResolver` so the
        // parser crate stays free of FS / NDN dependencies.
        if let Err(reason) = validator.validate_path(raw_path) {
            return Ok(AttachmentLowering::Rejected(reason));
        }
        return Ok(AttachmentLowering::PendingPath(raw_path.clone()));
    }
    let Some(raw_obj_id) = tag.obj_id.as_ref().filter(|s| !s.trim().is_empty()) else {
        return Ok(AttachmentLowering::Keep);
    };
    let obj_id = ObjId::new(raw_obj_id).map_err(|err| MsgParserError::InvalidObjId {
        value: raw_obj_id.clone(),
        message: err.to_string(),
    })?;
    if let Err(reason) = validator.validate_obj_id(&obj_id) {
        return Ok(AttachmentLowering::Rejected(reason));
    }
    Ok(AttachmentLowering::Ref(RefItem {
        role: RefRole::Input,
        target: RefTarget::DataObj {
            obj_id,
            uri_hint: None,
        },
        label: tag.title.clone().or_else(|| tag.label.clone()),
    }))
}

fn parse_attachment_tag(raw: &str) -> Option<AttachmentTag> {
    let raw = raw.trim();
    let after_open = raw.strip_prefix("<attachment")?;

    // Two recognized shapes on a single line:
    //   self-closing: `<attachment ...attrs... />`  or  `<attachment>`(no attrs)
    //   body form:    `<attachment ...attrs...>BODY</attachment>`
    //
    // Body content lets the LLM write the more natural
    // `<attachment>/abs/path.html</attachment>` — we sniff it as a CYFS
    // ObjId first, fall back to a local filesystem path. This is the
    // primary surface taught to agents; the attribute form remains
    // available for callers that build tags programmatically.
    let (attrs_raw, body_text): (&str, Option<&str>) =
        if let Some(rest) = after_open.strip_suffix("</attachment>") {
            let open_close = rest.find('>')?;
            (
                rest[..open_close].trim(),
                Some(rest[open_close + 1..].trim()),
            )
        } else {
            let rest = after_open.strip_suffix('>')?;
            (rest.strip_suffix('/').unwrap_or(rest).trim(), None)
        };

    let attrs = parse_attrs(attrs_raw);
    let mut obj_id = attrs.get("obj_id").cloned();
    let mut path = attrs.get("path").cloned();
    let url = attrs.get("url").cloned();

    if let Some(body) = body_text.filter(|s| !s.is_empty()) {
        if obj_id.is_none() && path.is_none() && url.is_none() {
            if attachment_body_looks_like_obj_id(body) {
                obj_id = Some(body.to_string());
            } else {
                path = Some(body.to_string());
            }
        }
    }

    Some(AttachmentTag {
        kind: attrs
            .get("kind")
            .cloned()
            .unwrap_or_else(|| "document".to_string()),
        obj_id,
        url,
        path,
        mime: attrs.get("mime").cloned(),
        title: attrs.get("title").cloned(),
        label: attrs.get("label").cloned(),
    })
}

fn attachment_body_looks_like_obj_id(body: &str) -> bool {
    // ObjId strings are content-addressed identifiers with no path
    // separators or whitespace. The real validation lives in
    // `attachment_tag_to_ref_item` via `ObjId::new`; this sniff just
    // routes `<attachment>BODY</attachment>` into the right slot. The
    // ObjId::new probe is the source of truth — anything that parses
    // is treated as an obj_id, everything else as a local path.
    if body.contains('/') || body.contains('\\') || body.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    ObjId::new(body).is_ok()
}

fn render_attachment_tag(tag: &AttachmentTag) -> String {
    let mut attrs = vec![format!("kind=\"{}\"", xml_escape(&tag.kind))];
    if let Some(obj_id) = &tag.obj_id {
        attrs.push(format!("obj_id=\"{}\"", xml_escape(obj_id)));
    }
    if let Some(url) = &tag.url {
        attrs.push(format!("source=\"url\""));
        attrs.push(format!("url=\"{}\"", xml_escape(url)));
    }
    if let Some(path) = &tag.path {
        attrs.push(format!("path=\"{}\"", xml_escape(path)));
    }
    if let Some(mime) = &tag.mime {
        attrs.push(format!("mime=\"{}\"", xml_escape(mime)));
    }
    if let Some(title) = &tag.title {
        attrs.push(format!("title=\"{}\"", xml_escape(title)));
    }
    if let Some(label) = &tag.label {
        attrs.push(format!("label=\"{}\"", xml_escape(label)));
    }
    format!("<attachment {} />", attrs.join(" "))
}

/// Render a rejected attachment as text so the recipient still sees the
/// LLM's intent and the reason it was filtered. The original tag is kept
/// for debugging; the leading `<!-- … -->` line makes the rejection obvious
/// in MsgObject.content without breaking surrounding markdown.
fn render_attachment_rejection(tag: &AttachmentTag, reason: &str) -> String {
    format!(
        "<!-- attachment rejected: {} -->\n{}",
        reason,
        render_attachment_tag(tag)
    )
}

fn machine_payloads_to_machine(payloads: Vec<Value>) -> Option<MachineContent> {
    if payloads.is_empty() {
        return None;
    }
    let mut data = BTreeMap::new();
    data.insert(
        "provider_state".to_string(),
        json_to_canon(Value::Array(payloads)),
    );
    Some(MachineContent {
        intent: Some(PROVIDER_MSG_MACHINE.to_string()),
        data,
    })
}

fn json_to_canon(value: Value) -> ndn_lib::CanonValue {
    match value {
        Value::Null => ndn_lib::CanonValue::Null,
        Value::Bool(v) => ndn_lib::CanonValue::Bool(v),
        Value::Number(n) => {
            if let Some(v) = n.as_i64() {
                ndn_lib::CanonValue::I64(v)
            } else if let Some(v) = n.as_u64() {
                ndn_lib::CanonValue::U64(v)
            } else if let Some(v) = n.as_f64() {
                ndn_lib::CanonValue::F64(v)
            } else {
                ndn_lib::CanonValue::Null
            }
        }
        Value::String(v) => ndn_lib::CanonValue::String(v),
        Value::Array(items) => {
            ndn_lib::CanonValue::Array(items.into_iter().map(json_to_canon).collect())
        }
        Value::Object(map) => ndn_lib::CanonValue::Object(
            map.into_iter()
                .map(|(k, v)| (k, json_to_canon(v)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use name_lib::DID;
    use ndn_lib::MsgObjKind;

    struct Allow;
    impl AttachmentValidator for Allow {}

    struct NoFiles;
    #[async_trait::async_trait]
    impl LocalFileResolver for NoFiles {
        async fn resolve(&self, path: &str) -> std::result::Result<ObjId, String> {
            Err(format!("no file {path}"))
        }
    }

    fn obj_id() -> ObjId {
        ObjId::new("file:010203").unwrap()
    }

    async fn egress(msg: &AiMessage, options: MsgEgressOptions) -> MsgObject {
        let base = MsgObject::new(
            DID::undefined(),
            vec![DID::undefined()],
            MsgObjKind::Chat,
            MsgContent::default(),
        );
        ai_message_to_msg_object_with_base_validated_async(msg, base, &Allow, options, &NoFiles)
            .await
            .unwrap()
    }

    #[test]
    fn attachment_kind_follows_label_then_format() {
        let inferred = ["image/png", "audio/mpeg", "clip.mp4", "archive.zip"]
            .into_iter()
            .map(|label| {
                let format = Some(&MsgContentFormat::TextPlain);
                (
                    attachment_kind(format, Some(label), None),
                    attachment_mime(format, Some(label), None).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            inferred,
            vec![
                ("image", "image/png".to_string()),
                ("audio", "audio/mpeg".to_string()),
                ("video", "video/mp4".to_string()),
                ("file", "application/zip".to_string()),
            ]
        );
        for (format, expected) in [
            (MsgContentFormat::AudioMpeg, "audio"),
            (MsgContentFormat::VideoMp4, "video"),
            (MsgContentFormat::ApplicationOctetStream, "file"),
            (MsgContentFormat::ApplicationPdf, "document"),
        ] {
            assert_eq!(attachment_kind(Some(&format), None, None), expected);
        }
    }

    #[tokio::test]
    async fn ai_message_named_object_content_becomes_msg_ref() {
        let msg = AiMessage::new(
            AiRole::Assistant,
            vec![
                AiContent::text("done"),
                AiContent::Document {
                    source: ResourceRef::named_object(obj_id()),
                    title: Some("report.pdf".to_string()),
                },
            ],
        );

        let out = egress(&msg, MsgEgressOptions::default()).await;
        assert_eq!(out.content.content, "done");
        assert_eq!(out.content.refs.len(), 1);
        assert_eq!(out.content.refs[0].label.as_deref(), Some("report.pdf"));
        assert_eq!(
            out.meta.get("llm_role"),
            Some(&Value::String("assistant".to_string()))
        );
    }

    #[tokio::test]
    async fn text_attachment_marker_becomes_msg_ref() {
        let msg = AiMessage::text(
            AiRole::Assistant,
            "see this\n<attachment kind=\"image\" obj_id=\"file:010203\" title=\"x.png\" />",
        );

        let out = egress(&msg, MsgEgressOptions::default()).await;
        assert_eq!(out.content.content, "see this");
        assert_eq!(out.content.refs.len(), 1);
        assert_eq!(out.content.refs[0].label.as_deref(), Some("x.png"));
    }

    #[test]
    fn attachment_tag_keeps_path_and_display_title_separate() {
        let tag = parse_attachment_tag(
            "<attachment path=\"/tmp/wusong_fights_tiger.svg\" title=\"武松打虎.svg\" />",
        )
        .unwrap();

        assert_eq!(tag.path.as_deref(), Some("/tmp/wusong_fights_tiger.svg"));
        assert_eq!(tag.title.as_deref(), Some("武松打虎.svg"));
        assert_eq!(tag.obj_id, None);
    }

    #[tokio::test]
    async fn text_attachment_marker_can_be_preserved_by_option() {
        let msg = AiMessage::text(
            AiRole::Assistant,
            "see this\n<attachment kind=\"image\" obj_id=\"file:010203\" title=\"x.png\" />",
        );

        let out = egress(
            &msg,
            MsgEgressOptions {
                preserve_attachment_tag_in_egress: true,
            },
        )
        .await;

        assert!(out.content.content.contains("<attachment"));
        assert_eq!(out.content.refs.len(), 1);
    }
}
