//! CYFS MessageHub <-> LLMContext message bridge.
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
//! The bridge keeps these rules deliberately narrow:
//!
//! 1. Text stays text. `MsgContent.content` becomes an `AiContent::Text`
//!    block, preserving its position before attachment blocks. Multiple text
//!    blocks from an LLM response are joined with blank lines when producing
//!    a `MsgObject`.
//! 2. CYFS data-object references become non-text LLM content. A `DataObj`
//!    ref is lowered to `AiContent::Image` when its MIME/label/URI looks like
//!    an image; otherwise it becomes `AiContent::Document`. The CYFS object id
//!    is preserved as `ResourceRef::NamedObject`, because that is the stable
//!    cross-zone identity.
//! 3. LLM non-text content is mapped back to MessageHub attachments whenever
//!    it can be represented by `MsgContent.refs`. `ResourceRef::NamedObject`
//!    becomes a `RefItem::DataObj`. Provider-only sources such as URL or
//!    base64 cannot be losslessly represented as a CYFS ref, so they are
//!    serialized into a small textual marker:
//!
//!    `<attachment kind="image" source="url" url="https://..." mime="image/png"/>`
//!
//!    The same marker is accepted inside `AiContent::Text`, allowing an LLM
//!    that can only emit text to request an outgoing attachment by object id:
//!
//!    `<attachment kind="document" obj_id="file:0123abcd" title="report.pdf"/>`
//!
//! 4. System-control commands are recognized before normal lowering through
//!    `parse_msg_object`. The rule is intentionally strict: a pure text
//!    message matching `^/(<registered-command>)(\s+<args>)?$`, with no refs
//!    and no machine payload, is returned as `MsgParseOutput::ControlCommand`.
//!    Slash-prefixed text whose command name is not in the caller-supplied
//!    registry falls through as a normal `AiMessage`. Callers that do not want
//!    control semantics can use `msg_object_to_ai_message` directly.
//!
//! This file should remain free of OpenDAN session policy. It only translates
//! protocol shapes; routing, authorization, command execution and reply policy
//! belong to the caller.

use std::collections::{BTreeMap, HashMap};

use buckyos_api::{AiContent, AiMessage, AiRole, ResourceRef};
use name_lib::DID;
use ndn_lib::{
    MachineContent, MsgContent, MsgContentFormat, MsgObjKind, MsgObject, ObjId, RefItem, RefRole,
    RefTarget,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PROVIDER_MSG_MACHINE: &str = "buckyos.msg.machine";
const PROVIDER_MSG_SERVICE_REF: &str = "buckyos.msg.ref.service_did";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemControlCommand {
    pub raw: String,
    pub command: String,
    pub args: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MsgParseOutput {
    ControlCommand(SystemControlCommand),
    Message(AiMessage),
}

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
/// implementation (workspace whitelist, ACL lookups, …); tests use the
/// permissive default.
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

/// Default validator that approves every attachment. Used by the
/// non-validated wrappers and in tests where no security boundary exists.
pub struct PermissiveAttachmentValidator;
impl AttachmentValidator for PermissiveAttachmentValidator {}

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

/// Parse a `MsgObject` into either a registered system-control command or a
/// normal `AiMessage`. Use this at MessageHub ingress points where
/// slash-commands should be intercepted before they reach the LLM.
pub fn parse_msg_object(msg: &MsgObject, registered_commands: &[&str]) -> MsgParseOutput {
    if let Some(command) = msg_object_control_command(msg, registered_commands) {
        MsgParseOutput::ControlCommand(command)
    } else {
        MsgParseOutput::Message(msg_object_to_ai_message(msg))
    }
}

/// Lower a `MsgObject` into a provider-neutral `AiMessage` using `User` role.
/// This function does not interpret `/` commands; callers that need control
/// semantics should use `parse_msg_object`.
pub fn msg_object_to_ai_message(msg: &MsgObject) -> AiMessage {
    msg_object_to_ai_message_with_role(msg, AiRole::User)
}

pub fn msg_object_to_ai_message_with_role(msg: &MsgObject, role: AiRole) -> AiMessage {
    let mut blocks = Vec::new();
    let text = msg.content.content.trim();
    if !text.is_empty() {
        blocks.push(AiContent::text(text.to_string()));
    }

    for item in &msg.content.refs {
        if let Some(block) = ref_item_to_ai_content(item, msg.content.format.as_ref()) {
            blocks.push(block);
        }
    }

    if let Some(machine) = &msg.content.machine {
        if let Ok(value) = serde_json::to_value(machine) {
            blocks.push(AiContent::ProviderState {
                provider: PROVIDER_MSG_MACHINE.to_string(),
                value,
            });
        }
    }

    if blocks.is_empty() {
        blocks.push(AiContent::text(String::new()));
    }
    AiMessage::new(role, blocks)
}

/// Convert an inferred LLM output message back into a `MsgObject`.
///
/// `from`, `to`, and `kind` are required because `AiMessage` intentionally has
/// no MessageHub envelope. Use `ai_message_to_msg_object_with_base` when the
/// caller already has a partially-filled `MsgObject` envelope.
pub fn ai_message_to_msg_object(
    message: &AiMessage,
    from: DID,
    to: Vec<DID>,
    kind: MsgObjKind,
) -> Result<MsgObject, MsgParserError> {
    let base = MsgObject::new(from, to, kind, MsgContent::default());
    ai_message_to_msg_object_with_base(message, base)
}

/// Convert an `AiMessage` into a `MsgObject`, preserving the caller-provided
/// envelope fields in `base` and replacing only `base.content`. Uses the
/// permissive validator — call `ai_message_to_msg_object_with_base_validated`
/// from policy-bearing surfaces (e.g. opendan egress) to enforce the §2.2.2
/// path / ACL whitelist.
pub fn ai_message_to_msg_object_with_base(
    message: &AiMessage,
    base: MsgObject,
) -> Result<MsgObject, MsgParserError> {
    ai_message_to_msg_object_with_base_validated(message, base, &PermissiveAttachmentValidator)
}

/// Same as [`ai_message_to_msg_object_with_base`], but every `<attachment>`
/// reference produced by the LLM (obj_id / path / url) is filtered through
/// `validator`. Rejected attachments are left inline as text so the model's
/// intent is preserved verbatim and the failure surfaces in audit logs,
/// rather than silently dropping a half-converted ref.
pub fn ai_message_to_msg_object_with_base_validated(
    message: &AiMessage,
    base: MsgObject,
    validator: &dyn AttachmentValidator,
) -> Result<MsgObject, MsgParserError> {
    ai_message_to_msg_object_with_base_validated_with_options(
        message,
        base,
        validator,
        MsgEgressOptions::default(),
    )
}

pub fn ai_message_to_msg_object_with_base_validated_with_options(
    message: &AiMessage,
    base: MsgObject,
    validator: &dyn AttachmentValidator,
    options: MsgEgressOptions,
) -> Result<MsgObject, MsgParserError> {
    let mut text_parts: Vec<String> = Vec::new();
    let mut refs: Vec<RefItem> = Vec::new();
    let mut machine_payloads: Vec<Value> = Vec::new();

    for block in &message.content {
        match block {
            AiContent::Text { text } => {
                collect_text_and_attachment_tags(
                    text,
                    &mut text_parts,
                    &mut refs,
                    validator,
                    options,
                )?;
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
            AiContent::ProviderState { provider, value } if provider == PROVIDER_MSG_MACHINE => {
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

/// Async counterpart of
/// [`ai_message_to_msg_object_with_base_validated_with_options`]. Routes
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
            AiContent::ProviderState { provider, value } if provider == PROVIDER_MSG_MACHINE => {
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

pub fn msg_object_control_command(
    msg: &MsgObject,
    registered_commands: &[&str],
) -> Option<SystemControlCommand> {
    if registered_commands.is_empty() {
        return None;
    }
    if !is_plain_text_format(msg.content.format.as_ref()) {
        return None;
    }
    if !msg.content.refs.is_empty() || msg.content.machine.is_some() {
        return None;
    }
    let raw = msg.content.content.as_str();
    if raw.is_empty() || !raw.starts_with('/') {
        return None;
    }
    let mut parts = raw[1..].splitn(2, char::is_whitespace);
    let command = parts.next().unwrap_or_default().trim().to_string();
    if command.is_empty() {
        return None;
    }
    let registered = registered_commands
        .iter()
        .map(|name| name.trim())
        .find(|name| !name.is_empty() && name.eq_ignore_ascii_case(&command))?;
    let args = parts.next().unwrap_or_default().trim().to_string();
    Some(SystemControlCommand {
        raw: raw.to_string(),
        command: registered.to_ascii_lowercase(),
        args,
    })
}

fn ref_item_to_ai_content(
    item: &RefItem,
    msg_format: Option<&MsgContentFormat>,
) -> Option<AiContent> {
    match &item.target {
        RefTarget::DataObj { obj_id, uri_hint } => {
            let source = ResourceRef::named_object(obj_id.clone());
            if looks_like_image(msg_format, item.label.as_deref(), uri_hint.as_deref()) {
                Some(AiContent::Image { source })
            } else {
                Some(AiContent::Document {
                    source,
                    title: item.label.clone(),
                })
            }
        }
        RefTarget::ServiceDid { did } => Some(AiContent::ProviderState {
            provider: PROVIDER_MSG_SERVICE_REF.to_string(),
            value: json!({
                "did": did.to_string(),
                "role": ref_role_name(item.role),
                "label": item.label,
            }),
        }),
    }
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
                escape_attr(kind),
                escape_attr(mime),
                escape_attr(data_base64),
                title
                    .map(|s| format!(" title=\"{}\"", escape_attr(s)))
                    .unwrap_or_default()
            ));
        }
    }
}

fn collect_text_and_attachment_tags(
    text: &str,
    text_parts: &mut Vec<String>,
    refs: &mut Vec<RefItem>,
    validator: &dyn AttachmentValidator,
    options: MsgEgressOptions,
) -> Result<(), MsgParserError> {
    let mut items = collect_egress_line_items(text, validator)?;
    // Sync path: no resolver wired. `PendingPath` collapses to the legacy
    // advisory-text rendering inside `finalize_egress_items_into`.
    resolve_pending_attachments_sync(&mut items);
    finalize_egress_items_into(items, text_parts, refs, options);
    Ok(())
}

/// Async variant of [`collect_text_and_attachment_tags`]. Pending local-file
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

fn resolve_pending_attachments_sync(items: &mut [EgressLineItem]) {
    for item in items.iter_mut() {
        if let EgressLineItem::Tag { lowering, .. } = item {
            if matches!(lowering, AttachmentLowering::PendingPath(_)) {
                *lowering = AttachmentLowering::Keep;
            }
        }
    }
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
        // parser crate stays free of FS / NDN dependencies; without a
        // resolver wired in, `PendingPath` collapses to `Keep`.
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

fn parse_attrs(raw: &str) -> HashMap<String, String> {
    let bytes = raw.as_bytes();
    let mut i = 0usize;
    let mut out = HashMap::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let key_start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'-'))
        {
            i += 1;
        }
        if i == key_start {
            break;
        }
        let key = &raw[key_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || !matches!(bytes[i], b'"' | b'\'') {
            continue;
        }
        let quote = bytes[i];
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        if i > value_start {
            out.insert(key.to_string(), unescape_attr(&raw[value_start..i]));
        } else {
            out.insert(key.to_string(), String::new());
        }
        if i < bytes.len() {
            i += 1;
        }
    }
    out
}

fn render_attachment_tag(tag: &AttachmentTag) -> String {
    let mut attrs = vec![format!("kind=\"{}\"", escape_attr(&tag.kind))];
    if let Some(obj_id) = &tag.obj_id {
        attrs.push(format!("obj_id=\"{}\"", escape_attr(obj_id)));
    }
    if let Some(url) = &tag.url {
        attrs.push(format!("source=\"url\""));
        attrs.push(format!("url=\"{}\"", escape_attr(url)));
    }
    if let Some(path) = &tag.path {
        attrs.push(format!("path=\"{}\"", escape_attr(path)));
    }
    if let Some(mime) = &tag.mime {
        attrs.push(format!("mime=\"{}\"", escape_attr(mime)));
    }
    if let Some(title) = &tag.title {
        attrs.push(format!("title=\"{}\"", escape_attr(title)));
    }
    if let Some(label) = &tag.label {
        attrs.push(format!("label=\"{}\"", escape_attr(label)));
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

fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn unescape_attr(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
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

fn is_plain_text_format(format: Option<&MsgContentFormat>) -> bool {
    matches!(
        format,
        None | Some(MsgContentFormat::TextPlain)
            | Some(MsgContentFormat::TextMarkdown)
            | Some(MsgContentFormat::TextHtml)
            | Some(MsgContentFormat::TextCss)
            | Some(MsgContentFormat::TextXml)
    )
}

fn looks_like_image(
    msg_format: Option<&MsgContentFormat>,
    label: Option<&str>,
    uri_hint: Option<&str>,
) -> bool {
    if msg_format.is_some_and(is_image_format) {
        return true;
    }
    label.is_some_and(looks_like_image_name) || uri_hint.is_some_and(looks_like_image_name)
}

fn is_image_format(format: &MsgContentFormat) -> bool {
    matches!(
        format,
        MsgContentFormat::ImagePng
            | MsgContentFormat::ImageJpeg
            | MsgContentFormat::ImageGif
            | MsgContentFormat::ImageWebp
            | MsgContentFormat::ImageSvg
            | MsgContentFormat::ImageBmp
    )
}

fn looks_like_image_name(value: &str) -> bool {
    let v = value.to_ascii_lowercase();
    v.starts_with("image/")
        || v.ends_with(".png")
        || v.ends_with(".jpg")
        || v.ends_with(".jpeg")
        || v.ends_with(".gif")
        || v.ends_with(".webp")
        || v.ends_with(".svg")
        || v.ends_with(".bmp")
}

fn ref_role_name(role: RefRole) -> &'static str {
    match role {
        RefRole::Context => "context",
        RefRole::Input => "input",
        RefRole::Output => "output",
        RefRole::Evidence => "evidence",
        RefRole::Control => "control",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj_id() -> ObjId {
        ObjId::new("file:010203").unwrap()
    }

    #[test]
    fn msg_object_lowers_text_and_image_ref() {
        let msg = MsgObject {
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: "look".to_string(),
                refs: vec![RefItem {
                    role: RefRole::Input,
                    target: RefTarget::DataObj {
                        obj_id: obj_id(),
                        uri_hint: Some("photo.png".to_string()),
                    },
                    label: Some("photo.png".to_string()),
                }],
                ..MsgContent::default()
            },
            ..MsgObject::default()
        };

        let out = msg_object_to_ai_message(&msg);
        assert_eq!(out.role, AiRole::User);
        assert!(matches!(out.content[0], AiContent::Text { .. }));
        assert!(matches!(out.content[1], AiContent::Image { .. }));
    }

    #[test]
    fn slash_text_matching_registered_name_is_control_command() {
        let msg = MsgObject {
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: "/switch coding".to_string(),
                ..MsgContent::default()
            },
            ..MsgObject::default()
        };

        match parse_msg_object(
            &msg,
            &[
                "new", "clean", "stop", "cancel", "info", "list", "switch", "help",
            ],
        ) {
            MsgParseOutput::ControlCommand(cmd) => {
                assert_eq!(cmd.command, "switch");
                assert_eq!(cmd.args, "coding");
            }
            other => panic!("expected control command, got {other:?}"),
        }
    }

    #[test]
    fn slash_text_not_in_registry_lowers_as_message() {
        let msg = MsgObject {
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: "/etc/nginx/conf 帮我看下".to_string(),
                ..MsgContent::default()
            },
            ..MsgObject::default()
        };

        match parse_msg_object(
            &msg,
            &[
                "new", "clean", "stop", "cancel", "info", "list", "switch", "help",
            ],
        ) {
            MsgParseOutput::Message(ai) => match &ai.content[0] {
                AiContent::Text { text } => {
                    assert_eq!(text, "/etc/nginx/conf 帮我看下");
                }
                other => panic!("expected text message, got {other:?}"),
            },
            other => panic!("expected normal message, got {other:?}"),
        }
    }

    #[test]
    fn ai_message_named_object_content_becomes_msg_ref() {
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

        let out = ai_message_to_msg_object(
            &msg,
            DID::undefined(),
            vec![DID::undefined()],
            MsgObjKind::Chat,
        )
        .unwrap();
        assert_eq!(out.content.content, "done");
        assert_eq!(out.content.refs.len(), 1);
        assert_eq!(out.content.refs[0].label.as_deref(), Some("report.pdf"));
        assert_eq!(
            out.meta.get("llm_role"),
            Some(&Value::String("assistant".to_string()))
        );
    }

    #[test]
    fn text_attachment_marker_becomes_msg_ref() {
        let msg = AiMessage::text(
            AiRole::Assistant,
            "see this\n<attachment kind=\"image\" obj_id=\"file:010203\" title=\"x.png\" />",
        );

        let out = ai_message_to_msg_object(
            &msg,
            DID::undefined(),
            vec![DID::undefined()],
            MsgObjKind::Chat,
        )
        .unwrap();
        assert_eq!(out.content.content, "see this");
        assert_eq!(out.content.refs.len(), 1);
        assert_eq!(out.content.refs[0].label.as_deref(), Some("x.png"));
    }

    #[test]
    fn text_attachment_marker_can_be_preserved_by_option() {
        let msg = AiMessage::text(
            AiRole::Assistant,
            "see this\n<attachment kind=\"image\" obj_id=\"file:010203\" title=\"x.png\" />",
        );
        let base = MsgObject::new(
            DID::undefined(),
            vec![DID::undefined()],
            MsgObjKind::Chat,
            MsgContent::default(),
        );

        let out = ai_message_to_msg_object_with_base_validated_with_options(
            &msg,
            base,
            &PermissiveAttachmentValidator,
            MsgEgressOptions {
                preserve_attachment_tag_in_egress: true,
            },
        )
        .unwrap();

        assert!(out.content.content.contains("<attachment"));
        assert_eq!(out.content.refs.len(), 1);
    }
}
