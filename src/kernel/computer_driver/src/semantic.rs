//! Semantic observation shared by every backend: nodes use the common role,
//! state and action names (`buckyos_api::ComputerRole` …), are pruned and
//! budgeted in breadth-first order, and are printed as `outline-v1` text.
//! The Windows UI Automation mapping to those names lives in [`uia`].
#![cfg_attr(not(windows), allow(dead_code))]

use buckyos_api::{
    ComputerBounds, ComputerElementAction, ComputerElementState, ComputerRole,
    COMPUTER_SEMANTIC_MAX_DEPTH, COMPUTER_SEMANTIC_MAX_NODES, COMPUTER_SEMANTIC_MAX_TEXT_BYTES,
};

const NAME_CHARS: usize = 160;
const VALUE_CHARS: usize = 120;
const ID_BYTES: usize = 64;

/// One backend node; `parent` precedes it (nodes come in breadth-first order).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SemanticNode {
    pub parent: Option<usize>,
    pub role: ComputerRole,
    pub name: String,
    pub id: String,
    pub value: Option<String>,
    pub states: Vec<ComputerElementState>,
    pub actions: Vec<ComputerElementAction>,
    pub bounds: ComputerBounds,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Limits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_text_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_nodes: COMPUTER_SEMANTIC_MAX_NODES,
            max_depth: COMPUTER_SEMANTIC_MAX_DEPTH,
            max_text_bytes: COMPUTER_SEMANTIC_MAX_TEXT_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outline {
    pub text: String,
    /// `(ref, node index)` in outline order.
    pub refs: Vec<(String, usize)>,
    pub actionable: usize,
    pub truncated: Option<&'static str>,
}

fn listed(action: &ComputerElementAction) -> bool {
    *action != ComputerElementAction::ScrollIntoView
}

fn prunable(node: &SemanticNode, anchor_name: Option<&str>) -> bool {
    let actionless = !node.actions.iter().any(listed);
    let plain = matches!(node.role, ComputerRole::Text | ComputerRole::Image);
    (node.name.is_empty()
        && actionless
        && (plain || matches!(node.role, ComputerRole::Generic | ComputerRole::Group)))
        || (plain
            && actionless
            && !node.name.is_empty()
            && anchor_name.is_some_and(|name| name.contains(node.name.as_str())))
}

/// Invisible format characters (zero-width spaces and joiners, bidi marks
/// and isolates, BOM, tags) that pad names without showing anything.
fn invisible(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// Single-line, length-bounded text for a quoted outline field.
fn clean(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut chars = 0;
    let mut space = false;
    for character in text.chars() {
        if invisible(character) {
            continue;
        }
        if character.is_whitespace() || character.is_control() {
            space = !out.is_empty();
            continue;
        }
        if chars == max_chars {
            out.push('…');
            return out;
        }
        if space {
            out.push(' ');
            chars += 1;
            space = false;
            if chars == max_chars {
                out.push('…');
                return out;
            }
        }
        if character == '"' || character == '\\' {
            out.push('\\');
        }
        out.push(character);
        chars += 1;
    }
    out
}

fn line(node: &SemanticNode) -> String {
    let mut line = node.role.as_str().to_string();
    let name = clean(&node.name, NAME_CHARS);
    if !name.is_empty() {
        line.push_str(" \"");
        line.push_str(&name);
        line.push('"');
    }
    if !node.id.is_empty()
        && node.id.len() <= ID_BYTES
        && node.id.chars().all(|c| c.is_ascii_graphic() && c != '"')
    {
        line.push_str(" #");
        line.push_str(&node.id);
    }
    if let Some(value) = node.value.as_deref().filter(|v| !v.is_empty() && *v != node.name) {
        line.push_str(" value=\"");
        line.push_str(&clean(value, VALUE_CHARS));
        line.push('"');
    }
    for state in &node.states {
        line.push(' ');
        line.push_str(state.as_str());
    }
    let actions: Vec<_> = node
        .actions
        .iter()
        .filter(|a| listed(a))
        .map(|a| a.as_str())
        .collect();
    if !actions.is_empty() {
        line.push_str(" (");
        line.push_str(&actions.join(", "));
        line.push(')');
    }
    let b = node.bounds;
    line.push_str(&format!(" @{},{} {}x{}", b.x, b.y, b.width, b.height));
    line
}

/// Prune nameless action-less containers (their children move up), drop
/// labels that repeat their ancestor's name, keep `max_nodes` / `max_depth`
/// / `max_text_bytes` in breadth-first order, then print in tree order.
pub(crate) fn outline(nodes: &[SemanticNode], limits: &Limits) -> Outline {
    let count = nodes.len();
    let mut kept = vec![false; count];
    let mut dropped = vec![false; count];
    let mut anchor = vec![None::<usize>; count];
    let mut depth = vec![0usize; count];
    let mut lines = vec![String::new(); count];
    let ref_width = 4 + limits.max_nodes.max(1).to_string().len();
    let (mut returned, mut bytes, mut stop) = (0usize, 0usize, false);
    let mut truncated = None;
    for i in 0..count {
        let parent = nodes[i].parent.filter(|p| *p < i);
        anchor[i] = parent.and_then(|p| if kept[p] { Some(p) } else { anchor[p] });
        if stop || parent.is_some_and(|p| dropped[p]) {
            dropped[i] = true;
            continue;
        }
        if parent.is_some() && prunable(&nodes[i], anchor[i].map(|a| nodes[a].name.as_str())) {
            continue;
        }
        let level = anchor[i].map_or(0, |a| depth[a] + 1);
        if level > limits.max_depth {
            dropped[i] = true;
            truncated.get_or_insert("max_depth");
            continue;
        }
        let text = line(&nodes[i]);
        let cost = 2 * level + ref_width + text.len() + 1;
        if returned >= limits.max_nodes || bytes + cost > limits.max_text_bytes {
            dropped[i] = true;
            stop = true;
            truncated.get_or_insert(if returned >= limits.max_nodes {
                "max_nodes"
            } else {
                "max_text_bytes"
            });
            continue;
        }
        kept[i] = true;
        depth[i] = level;
        lines[i] = text;
        bytes += cost;
        returned += 1;
    }
    let mut children = vec![Vec::new(); count];
    let mut roots = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        match node.parent.filter(|p| *p < i) {
            Some(parent) => children[parent].push(i),
            None => roots.push(i),
        }
    }
    let mut stack: Vec<usize> = roots.into_iter().rev().collect();
    let mut outline = Outline {
        text: String::with_capacity(bytes),
        refs: Vec::with_capacity(returned),
        actionable: 0,
        truncated,
    };
    while let Some(i) = stack.pop() {
        stack.extend(children[i].iter().rev());
        if !kept[i] {
            continue;
        }
        let reference = format!("e{}", outline.refs.len() + 1);
        for _ in 0..depth[i] {
            outline.text.push_str("  ");
        }
        outline.text.push('[');
        outline.text.push_str(&reference);
        outline.text.push_str("] ");
        outline.text.push_str(&lines[i]);
        outline.text.push('\n');
        if nodes[i].actions.iter().any(listed) {
            outline.actionable += 1;
        }
        outline.refs.push((reference, i));
    }
    outline
}

/// Window titles and process names for the window list.
pub(crate) fn plain(text: &str) -> String {
    clean(text, NAME_CHARS).replace("\\\"", "\"").replace("\\\\", "\\")
}

/// Windows UI Automation: control types, patterns and states mapped to the
/// common names. Pattern/state bits match `native_windows.cpp`.
pub(crate) mod uia {
    use super::*;
    use serde::Deserialize;

    pub const PAT_INVOKE: u32 = 1 << 0;
    pub const PAT_VALUE: u32 = 1 << 1;
    pub const PAT_TOGGLE: u32 = 1 << 2;
    pub const PAT_SELECTION_ITEM: u32 = 1 << 3;
    pub const PAT_EXPAND_COLLAPSE: u32 = 1 << 4;
    pub const PAT_SCROLL_ITEM: u32 = 1 << 5;
    pub const PAT_FOCUSABLE: u32 = 1 << 6;

    pub const ST_DISABLED: u32 = 1 << 0;
    pub const ST_FOCUSED: u32 = 1 << 1;
    pub const ST_READONLY: u32 = 1 << 2;
    pub const ST_SELECTED: u32 = 1 << 3;
    pub const ST_CHECKED: u32 = 1 << 4;
    pub const ST_MIXED: u32 = 1 << 5;
    pub const ST_EXPANDED: u32 = 1 << 6;
    pub const ST_COLLAPSED: u32 = 1 << 7;
    pub const ST_PARTIAL: u32 = 1 << 8;
    pub const ST_DIALOG: u32 = 1 << 9;

    #[derive(Debug, Clone, Deserialize)]
    pub struct Window {
        pub hwnd: u64,
        pub pid: u32,
        pub title: String,
        pub process: String,
        pub bounds: [i64; 4],
        pub foreground: bool,
        pub accessible: bool,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct Node {
        pub p: i64,
        pub ct: i32,
        pub n: String,
        pub aid: String,
        pub cls: String,
        pub aria: String,
        pub hl: i32,
        pub pat: u32,
        pub st: u32,
        pub b: [i64; 4],
        #[serde(default)]
        pub v: Option<String>,
    }

    #[derive(Debug, Clone, Deserialize)]
    pub struct Snapshot {
        pub root: u64,
        pub root_pid: u32,
        pub windows: Vec<Window>,
        pub nodes: Vec<Node>,
        pub visited: u32,
        pub truncated: bool,
        pub reason: String,
        pub snapshot: u64,
    }

    fn aria_role(aria: &str) -> Option<ComputerRole> {
        use ComputerRole::*;
        Some(match aria {
            "button" => Button,
            "link" => Link,
            "tab" => Tab,
            "tablist" => Tablist,
            "menu" => Menu,
            "menubar" => Menubar,
            "menuitem" | "menuitemcheckbox" | "menuitemradio" => Menuitem,
            "checkbox" => Checkbox,
            "radio" => Radio,
            "switch" => Switch,
            "textbox" | "searchbox" | "spinbutton" => Textbox,
            "combobox" => Combobox,
            "slider" => Slider,
            "list" | "listbox" => List,
            "listitem" | "option" => Listitem,
            "tree" => Tree,
            "treeitem" => Treeitem,
            "table" | "grid" | "treegrid" => Table,
            "row" => Row,
            "cell" | "gridcell" | "columnheader" | "rowheader" => Cell,
            "heading" => Heading,
            "dialog" | "alertdialog" => Dialog,
            "toolbar" => Toolbar,
            "img" | "image" => Image,
            "document" => Document,
            "group" | "region" | "navigation" | "main" | "banner" | "complementary"
            | "contentinfo" | "form" | "search" | "article" | "radiogroup" | "tabpanel"
            | "rowgroup" => Group,
            _ => return None,
        })
    }

    pub fn role(control_type: i32, aria: &str, class: &str, heading: i32, states: u32, patterns: u32) -> ComputerRole {
        use ComputerRole::*;
        if let Some(role) = aria_role(&aria.to_ascii_lowercase()) {
            return role;
        }
        match control_type {
            50000 if patterns & PAT_TOGGLE != 0 && class == "ToggleSwitch" => Switch,
            50000 | 50031 => Button,
            50002 => Checkbox,
            50003 => Combobox,
            50004 => Textbox,
            50005 => Link,
            50006 => Image,
            50007 => Listitem,
            50008 => List,
            50009 => Menu,
            50010 => Menubar,
            50011 => Menuitem,
            50013 => Radio,
            50015 => Slider,
            50018 => Tablist,
            50019 => Tab,
            50020 | 50025 | 50026 | 50033 if heading > 0 => Heading,
            50020 => Text,
            50021 | 50040 => Toolbar,
            50023 => Tree,
            50024 => Treeitem,
            50028 | 50036 => Table,
            50029 => Row,
            50030 => Document,
            50032 if states & ST_DIALOG != 0 => Dialog,
            50032 => Window,
            50035 => Cell,
            50001 | 50017 | 50026 | 50034 => Group,
            _ => Generic,
        }
    }

    pub fn states(role: ComputerRole, bits: u32) -> Vec<ComputerElementState> {
        use ComputerElementState::*;
        let mut states = Vec::new();
        for (bit, state) in [
            (ST_DISABLED, Disabled),
            (ST_FOCUSED, Focused),
            (ST_SELECTED, Selected),
            (ST_CHECKED, Checked),
            (ST_MIXED, Mixed),
            (ST_EXPANDED | ST_PARTIAL, Expanded),
            (ST_COLLAPSED, Collapsed),
        ] {
            if bits & bit != 0 {
                states.push(state);
            }
        }
        if bits & ST_READONLY != 0 && matches!(role, ComputerRole::Textbox | ComputerRole::Combobox) {
            states.push(Readonly);
        }
        states
    }

    pub fn actions(patterns: u32, states: u32) -> Vec<ComputerElementAction> {
        use ComputerElementAction::*;
        let mut actions = Vec::new();
        for (bit, action) in [
            (PAT_FOCUSABLE, Focus),
            (PAT_INVOKE, Invoke),
            (PAT_TOGGLE, Toggle),
            (PAT_SELECTION_ITEM, Select),
            (PAT_SCROLL_ITEM, ScrollIntoView),
        ] {
            if patterns & bit != 0 {
                actions.push(action);
            }
        }
        if patterns & PAT_VALUE != 0 && states & ST_READONLY == 0 {
            actions.push(SetValue);
        }
        if patterns & PAT_EXPAND_COLLAPSE != 0 {
            if states & (ST_COLLAPSED | ST_PARTIAL) != 0 {
                actions.push(Expand);
            }
            if states & (ST_EXPANDED | ST_PARTIAL) != 0 {
                actions.push(Collapse);
            }
        }
        actions.sort();
        actions
    }

    pub fn node(raw: &Node, bounds: ComputerBounds) -> SemanticNode {
        let role = role(raw.ct, &raw.aria, &raw.cls, raw.hl, raw.st, raw.pat);
        SemanticNode {
            parent: usize::try_from(raw.p).ok(),
            role,
            name: raw.n.clone(),
            id: raw.aid.clone(),
            value: raw
                .v
                .clone()
                .filter(|_| matches!(role, ComputerRole::Textbox | ComputerRole::Combobox | ComputerRole::Slider)),
            states: states(role, raw.st),
            actions: actions(raw.pat, raw.st),
            bounds,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::uia::*;
    use super::*;
    use ComputerElementAction as A;
    use ComputerRole as R;

    fn node(parent: Option<usize>, role: ComputerRole, name: &str, actions: &[A]) -> SemanticNode {
        SemanticNode {
            parent,
            role,
            name: name.into(),
            id: String::new(),
            value: None,
            states: vec![],
            actions: actions.to_vec(),
            bounds: ComputerBounds {
                x: 1,
                y: 2,
                width: 30,
                height: 4,
            },
        }
    }

    #[test]
    fn uia_control_types_map_to_common_roles() {
        assert_eq!(role(50019, "", "", 0, 0, 0), R::Tab);
        assert_eq!(role(50018, "", "", 0, 0, 0), R::Tablist);
        assert_eq!(role(50004, "", "", 0, 0, 0), R::Textbox);
        assert_eq!(role(50005, "", "", 0, 0, 0), R::Link);
        assert_eq!(role(50029, "", "", 0, 0, 0), R::Row);
        assert_eq!(role(50029, "gridcell", "", 0, 0, 0), R::Cell);
        assert_eq!(role(50000, "", "ToggleSwitch", 0, 0, PAT_TOGGLE), R::Switch);
        assert_eq!(role(50000, "", "Button", 0, 0, PAT_TOGGLE), R::Button);
        assert_eq!(role(50002, "switch", "", 0, 0, PAT_TOGGLE), R::Switch);
        assert_eq!(role(50020, "", "", 2, 0, 0), R::Heading);
        assert_eq!(role(50020, "", "", 0, 0, 0), R::Text);
        assert_eq!(role(50032, "", "", 0, ST_DIALOG, 0), R::Dialog);
        assert_eq!(role(50032, "", "", 0, 0, 0), R::Window);
        assert_eq!(role(50033, "", "", 0, 0, 0), R::Generic);
        assert_eq!(role(50033, "Navigation", "", 0, 0, 0), R::Group);
        assert_eq!(role(50025, "presentation", "", 0, 0, 0), R::Generic);
        assert_eq!(role(59999, "", "", 0, 0, 0), R::Generic);
        let containers = [
            50001, 50006, 50012, 50014, 50016, 50017, 50020, 50022, 50025, 50026, 50027, 50033,
            50034, 50037, 50038, 50039,
        ];
        for control_type in 50000..=50040 {
            let mapped = role(control_type, "", "", 0, 0, 0);
            assert_eq!(
                matches!(mapped, R::Generic | R::Group | R::Text | R::Image),
                containers.contains(&control_type),
                "native container_type() must agree for {control_type}"
            );
        }
    }

    #[test]
    fn uia_patterns_and_states_map_to_common_actions() {
        assert_eq!(
            actions(PAT_INVOKE | PAT_FOCUSABLE | PAT_SCROLL_ITEM, 0),
            vec![A::Focus, A::Invoke, A::ScrollIntoView]
        );
        assert_eq!(actions(PAT_VALUE, 0), vec![A::SetValue]);
        assert!(actions(PAT_VALUE, ST_READONLY).is_empty());
        assert_eq!(actions(PAT_EXPAND_COLLAPSE, ST_COLLAPSED), vec![A::Expand]);
        assert_eq!(actions(PAT_EXPAND_COLLAPSE, ST_EXPANDED), vec![A::Collapse]);
        assert_eq!(actions(PAT_EXPAND_COLLAPSE, ST_PARTIAL), vec![A::Expand, A::Collapse]);
        assert_eq!(actions(PAT_TOGGLE | PAT_SELECTION_ITEM, 0), vec![A::Toggle, A::Select]);
        assert_eq!(
            states(R::Checkbox, ST_CHECKED | ST_DISABLED | ST_FOCUSED),
            vec![
                ComputerElementState::Disabled,
                ComputerElementState::Focused,
                ComputerElementState::Checked
            ]
        );
        assert_eq!(states(R::Link, ST_READONLY), vec![]);
        assert_eq!(states(R::Textbox, ST_READONLY), vec![ComputerElementState::Readonly]);
        assert_eq!(states(R::Treeitem, ST_PARTIAL), vec![ComputerElementState::Expanded]);
    }

    #[test]
    fn uia_values_are_shown_only_for_editable_roles() {
        let raw: Node = serde_json::from_value(serde_json::json!({
            "p":0,"ct":50005,"n":"Spam","aid":"","cls":"","aria":"link","hl":0,"pat":PAT_VALUE|PAT_INVOKE,
            "st":ST_READONLY,"b":[0,0,10,10],"v":"https://mail.google.com/#spam","pid":7,"hwnd":9
        }))
        .unwrap();
        let link = uia::node(&raw, ComputerBounds::default());
        assert_eq!(link.parent, Some(0));
        assert_eq!(link.value, None);
        assert_eq!(link.actions, vec![A::Invoke]);
        let edit = uia::node(
            &Node {
                ct: 50004,
                aria: String::new(),
                st: 0,
                v: Some("draft".into()),
                ..raw
            },
            ComputerBounds::default(),
        );
        assert_eq!(edit.value.as_deref(), Some("draft"));
        assert_eq!(edit.actions, vec![A::Invoke, A::SetValue]);
    }

    #[test]
    fn outline_prunes_containers_lifts_children_and_keeps_tree_order() {
        let mut nodes = vec![
            node(None, R::Window, "Gmail - Chrome", &[]),
            node(Some(0), R::Generic, "", &[]),
            node(Some(0), R::Button, "删除", &[A::Invoke]),
            node(Some(1), R::Tab, "Gmail", &[A::Select]),
            node(Some(3), R::Text, "Gmail", &[]),
            node(Some(1), R::Link, "垃圾邮件 3", &[A::Invoke, A::ScrollIntoView]),
            node(Some(5), R::Group, "", &[A::ScrollIntoView]),
            node(Some(6), R::Text, "垃圾邮件", &[]),
            node(Some(1), R::Text, "Inbox", &[]),
        ];
        nodes[3].states = vec![ComputerElementState::Selected];
        nodes[3].id = "tab-1".into();
        let out = outline(&nodes, &Limits::default());
        assert_eq!(
            out.text,
            "[e1] window \"Gmail - Chrome\" @1,2 30x4\n\
             \x20 [e2] tab \"Gmail\" #tab-1 selected (select) @1,2 30x4\n\
             \x20 [e3] link \"垃圾邮件 3\" (invoke) @1,2 30x4\n\
             \x20 [e4] text \"Inbox\" @1,2 30x4\n\
             \x20 [e5] button \"删除\" (invoke) @1,2 30x4\n"
        );
        assert_eq!(
            out.refs,
            vec![
                ("e1".into(), 0),
                ("e2".into(), 3),
                ("e3".into(), 5),
                ("e4".into(), 8),
                ("e5".into(), 2)
            ]
        );
        assert_eq!(out.actionable, 3);
        assert_eq!(out.truncated, None);
    }

    #[test]
    fn outline_budgets_are_breadth_first_and_reported() {
        let mut nodes = vec![node(None, R::Window, "w", &[])];
        for i in 0..10 {
            nodes.push(node(Some(i), R::Button, &format!("b{i}"), &[A::Invoke]));
        }
        let out = outline(
            &nodes,
            &Limits {
                max_nodes: 4,
                ..Limits::default()
            },
        );
        assert_eq!(out.refs.len(), 4);
        assert_eq!(out.truncated, Some("max_nodes"));
        let deep = outline(
            &nodes,
            &Limits {
                max_depth: 2,
                ..Limits::default()
            },
        );
        assert_eq!(deep.refs.len(), 3);
        assert!(deep.text.lines().last().unwrap().starts_with("    [e3] button \"b1\""));
        assert_eq!(deep.truncated, Some("max_depth"));
        let small = outline(
            &nodes,
            &Limits {
                max_text_bytes: 120,
                ..Limits::default()
            },
        );
        assert!(small.text.len() <= 120);
        assert_eq!(small.truncated, Some("max_text_bytes"));
        assert!(!small.refs.is_empty());
    }

    #[test]
    fn outline_fields_are_single_line_escaped_and_bounded() {
        let mut long = node(None, R::Textbox, "Say \"hi\"\n\tnow \\ ok", &[A::SetValue]);
        long.value = Some("x".repeat(500));
        long.id = "has space".into();
        long.bounds = ComputerBounds {
            x: -8,
            y: 0,
            width: 1,
            height: 1,
        };
        let text = outline(&[long], &Limits::default()).text;
        assert!(text.starts_with("[e1] textbox \"Say \\\"hi\\\" now \\\\ ok\" value=\""));
        assert!(text.contains(&format!("{}…\"", "x".repeat(VALUE_CHARS))));
        assert!(!text.contains('#'));
        assert!(text.ends_with(" (set_value) @-8,0 1x1\n"));
        assert_eq!(text.lines().count(), 1);
        assert_eq!(clean(&"字".repeat(200), 3), "字字字…");
        assert_eq!(
            clean("Meet singles near yo\u{200C}\u{200D}\u{FEFF}\u{200B}u — ❤\u{FE0F}", 160),
            "Meet singles near you — ❤\u{FE0F}"
        );
        assert_eq!(clean("\u{200E}11/\u{200E}8/2026 \u{200F}\u{200E}10:22 PM", 160), "11/8/2026 10:22 PM");
        assert_eq!(plain("a \"b\"\n c"), "a \"b\" c");
    }
}
