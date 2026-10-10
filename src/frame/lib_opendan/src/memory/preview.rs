//! History preview of Memory material (component stage). The format is not
//! frozen; it shows what a Runtime would attach before an inference, clearly
//! labelled as Memory material and never as a user message or a system
//! instruction. Layer, state, scope and evidence character stay visible.

use std::collections::BTreeMap;

use super::changes::{Change, ChangeKind};
use super::hint::{Hint, HintLayer, HintState};
use super::recall::TopicResult;
use super::{Basis, DispositionOutcome, Scope};

/// Optional display names for references (`item_c1` → `c1`).
#[derive(Clone, Debug, Default)]
pub struct Names(pub BTreeMap<String, String>);

impl Names {
    pub fn name(&self, id: &str) -> String {
        self.0.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    fn reference(&self, h: &Hint) -> String {
        match h.revision {
            Some(r) => format!("{}@{r}", self.name(&h.id)),
            None => self.name(&h.id),
        }
    }

    fn cite(&self, c: &str) -> String {
        match c.split_once('@') {
            Some((id, r)) => format!("{}@{r}", self.name(id)),
            None => self.name(c),
        }
    }
}

fn state_label(s: HintState) -> &'static str {
    match s {
        HintState::Active => "有效",
        HintState::Disputed => "有分歧",
        HintState::ReviewPending => "待复核",
        HintState::MayBeCorrected => "可能已被纠正",
        HintState::Superseded => "已替代",
        HintState::Stale => "待复核或过期",
        HintState::Deleted => "已撤回",
        HintState::Pending => "待整理",
        HintState::Deferred => "暂缓整理",
        HintState::Disposed => "已处置",
    }
}

fn basis_label(b: Option<Basis>) -> &'static str {
    match b {
        Some(Basis::UserStatement) => "用户明确表达",
        Some(Basis::ToolObservation) => "工具观察",
        Some(Basis::ThirdPartyClaim) => "第三方主张",
        Some(Basis::Inference) => "推断",
        None => "未标注",
    }
}

pub fn scope_text(s: Option<&Scope>) -> String {
    let Some(s) = s else {
        return "未限定".to_string();
    };
    let mut t = s.subjects.join(", ");
    if !s.objects.is_empty() {
        t.push_str(" / ");
        t.push_str(&s.objects.join(", "));
    }
    if !s.exceptions.is_empty() {
        t.push_str(&format!("（不适用：{}）", s.exceptions.join(", ")));
    }
    t
}

fn sources(names: &Names, h: &Hint) -> String {
    if h.sources.is_empty() {
        "未记录".to_string()
    } else {
        h.sources
            .iter()
            .map(|s| names.name(s))
            .collect::<Vec<_>>()
            .join("、")
    }
}

/// One hint as a Memory block.
pub fn hint_block(names: &Names, h: &Hint) -> String {
    let mut out = String::new();
    match h.layer {
        HintLayer::Cognition => {
            let kind = h.kind.as_deref().unwrap_or("认知");
            out.push_str(&format!(
                "[认知 {}｜{}｜{}｜依据：{}]\n",
                names.reference(h),
                state_label(h.state),
                kind,
                basis_label(h.basis)
            ));
            out.push_str(&h.summary);
            out.push('\n');
            out.push_str(&format!(
                "范围：{}。来源：{}。\n",
                scope_text(h.scope.as_ref()),
                sources(names, h)
            ));
            if !h.corrections.is_empty() {
                out.push_str(&format!(
                    "注意：同一主体和范围下有尚未整理的纠正 {}，本条可能已不是当前要求。\n",
                    h.corrections
                        .iter()
                        .map(|c| names.name(c))
                        .collect::<Vec<_>>()
                        .join("、")
                ));
            }
            if !h.review_when.is_empty() {
                out.push_str(&format!("复核条件：{}。\n", h.review_when.join("；")));
            }
        }
        HintLayer::Perception => {
            if h.state == HintState::Disposed {
                return disposed_block(names, h);
            }
            let label = match (h.kind.as_deref(), h.explicit) {
                (Some("correction"), _) => "待整理纠正".to_string(),
                (_, true) => format!("{}｜用户明确要求记住", state_label(h.state)),
                _ => state_label(h.state).to_string(),
            };
            out.push_str(&format!(
                "[感知 {}｜{}｜记录于 {}]\n",
                names.reference(h),
                label,
                h.observed_at.clone().unwrap_or_default()
            ));
            out.push_str(&h.summary);
            out.push('\n');
            out.push_str(&format!(
                "范围：{}。原始事件：{}。",
                scope_text(h.scope.as_ref()),
                sources(names, h)
            ));
            if !h.cites.is_empty() {
                out.push_str(&format!(
                    "关联认知：{}。",
                    h.cites
                        .iter()
                        .map(|c| names.cite(c))
                        .collect::<Vec<_>>()
                        .join("、")
                ));
            }
            out.push('\n');
            out.push_str("这是尚未整理的观察，不等同于已确认的认知。\n");
        }
        HintLayer::Clarification => {
            out.push_str(&format!(
                "[待澄清｜低优先级｜材料 {}]\n",
                names.reference(h)
            ));
            out.push_str(&h.summary);
            out.push_str("\n是否询问由当前会话决定；不问也不影响当前任务。\n");
        }
    }
    if !h.reasons.is_empty() {
        out.push_str(&format!("相关原因：{}。\n", h.reasons.join("，")));
    }
    out
}

fn disposed_block(names: &Names, h: &Hint) -> String {
    let d = h.disposition.as_ref();
    let carried = d
        .map(|d| {
            d.cognition_refs
                .iter()
                .map(|c| names.cite(c))
                .collect::<Vec<_>>()
                .join("、")
        })
        .unwrap_or_default();
    let line = match d.map(|d| d.outcome) {
        Some(DispositionOutcome::Absorbed) => {
            format!("此前的感知 {} 已被 {carried} 吸收", names.reference(h))
        }
        Some(DispositionOutcome::Duplicate) => format!(
            "此前的感知 {} 已作为重复材料关联到 {carried}",
            names.reference(h)
        ),
        Some(DispositionOutcome::Discarded) => format!(
            "此前的感知 {} 已丢弃：{}",
            names.reference(h),
            d.and_then(|d| d.reason.clone()).unwrap_or_default()
        ),
        Some(DispositionOutcome::Deferred) => format!("此前的感知 {} 暂缓整理", names.reference(h)),
        None => format!("此前的感知 {} 已处置", names.reference(h)),
    };
    let cleaned = if h.summary.is_empty() {
        "，感知正文已清理"
    } else {
        ""
    };
    format!("[材料状态变化]\n{line}{cleaned}。后续使用引用承接它的认知；这是同一材料的整理结果，不是新的独立证据。\n")
}

/// The block a recall result adds to the next model input.
pub fn render_topic(names: &Names, title: &str, r: &TopicResult) -> String {
    let mut out = format!(
        "[Memory 线索：{title}]\n以下是与当前任务相关的历史材料；适用范围和证据性质随条目给出。\n"
    );
    for h in r.hints() {
        out.push('\n');
        out.push_str(&hint_block(names, h));
    }
    if r.pending_in_scope > 0 {
        out.push_str(&format!(
            "\n（本范围共有 {} 条尚未整理的感知。）\n",
            r.pending_in_scope
        ));
    }
    out
}

/// The block an observation adds to the next model input.
pub fn render_changes(names: &Names, title: &str, changes: &[Change]) -> String {
    let mut out = format!("[Memory 半订阅变化：{title}]\n");
    for c in changes {
        out.push('\n');
        match &c.kind {
            ChangeKind::ReviewPending { by } => {
                out.push_str(&format!(
                    "[旧认知待复核 {}]\n你此前收到的这条认知出现了同一主体的明确纠正 {}，在整理完成前不能继续作为当前默认要求。\n",
                    names.reference(&c.hint),
                    by.iter().map(|b| names.name(b)).collect::<Vec<_>>().join("、")
                ));
            }
            ChangeKind::ReadRevision { from } => {
                out.push_str(&format!(
                    "[Memory 已读认知修订｜{}@{from} → {}]\n",
                    names.name(&c.hint.id),
                    names.reference(&c.hint)
                ));
                out.push_str(&hint_block(names, &c.hint));
                out.push_str(&format!(
                    "显露原因：你此前读过 {}@{from}。\n",
                    names.name(&c.hint.id)
                ));
            }
            ChangeKind::RevisedCognition { from } => {
                out.push_str(&format!("（替代 {}@{from}）\n", names.name(&c.hint.id)));
                out.push_str(&hint_block(names, &c.hint));
            }
            _ => out.push_str(&hint_block(names, &c.hint)),
        }
    }
    out
}
