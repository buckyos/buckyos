//! Member names (archive TODO §2.7, D11, D12): one encoding per archive for the
//! names without a UTF-8 flag, separators handled after decoding, and the
//! addressable tree built from the decoded paths. Identity stays on the raw
//! bytes: a node's `raw` is its raw segments joined with `/`.

use crate::fsutil::validate_name;
use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{Encoding, BIG5, EUC_JP, EUC_KR, GBK, SHIFT_JIS};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Detection candidates, in the order paths fall back through them.
pub const CANDIDATES: [&Encoding; 5] = [GBK, BIG5, SHIFT_JIS, EUC_KR, EUC_JP];
const DETECT_ALONE: usize = 16;
const DETECT_WITH_HINT: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Utf8,
    Cp437,
    Enc(&'static Encoding),
}

impl Codec {
    pub fn name(&self) -> &'static str {
        match self {
            Codec::Utf8 => "UTF-8",
            Codec::Cp437 => "IBM437",
            Codec::Enc(e) => e.name(),
        }
    }

    pub fn decode(&self, raw: &[u8]) -> Option<String> {
        match self {
            Codec::Utf8 => std::str::from_utf8(raw).ok().map(String::from),
            Codec::Cp437 => Some(cp437(raw)),
            Codec::Enc(e) => e.decode_without_bom_handling_and_without_replacement(raw).map(|c| c.into_owned()),
        }
    }

    /// Whether the separator byte at `at` is a whole character, not the trail
    /// byte of a double-byte one (CP932 `表` is `95 5C`).
    fn whole_char_at(&self, raw: &[u8], at: usize) -> bool {
        match self {
            Codec::Utf8 | Codec::Cp437 => true,
            Codec::Enc(e) => e.decode_without_bom_handling_and_without_replacement(&raw[..at]).is_some(),
        }
    }
}

const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ',
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐',
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀',
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// What the `zip` crate shows for a name without the UTF-8 flag.
pub fn cp437(raw: &[u8]) -> String {
    raw.iter().map(|&b| if b < 0x80 { b as char } else { CP437_HIGH[(b - 0x80) as usize] }).collect()
}

/// The encoding preference of an `Accept-Language` header: its first CJK language.
pub fn preference(accept_language: &str) -> Option<&'static Encoding> {
    for part in accept_language.split(',') {
        let tag = part.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
        let mut sub = tag.split(['-', '_']);
        let lang = sub.next().unwrap_or("");
        let rest: Vec<&str> = sub.collect();
        match lang {
            "zh" => {
                let traditional = rest.iter().any(|s| matches!(*s, "tw" | "hk" | "mo" | "hant"));
                return Some(if traditional { BIG5 } else { GBK });
            }
            "ja" => return Some(SHIFT_JIS),
            "ko" => return Some(EUC_KR),
            _ => {}
        }
    }
    None
}

/// A central directory name: the raw bytes, and the decoded name when the
/// entry carries the UTF-8 flag or a valid Info-ZIP Unicode Path field.
#[derive(Debug, Clone)]
pub struct RawName {
    pub raw: Vec<u8>,
    pub unicode: Option<String>,
}

/// How an archive's names decode (§2.7 step 4).
#[derive(Debug, Clone)]
pub enum Decision {
    Fixed { codec: Codec, by: &'static str },
    /// Short names with several possible encodings: the requester's preference
    /// decides when it is one of `candidates`, `otherwise` decides the rest.
    Preference { candidates: Vec<&'static Encoding>, otherwise: (Codec, &'static str) },
}

impl Decision {
    pub fn for_request(&self, pref: Option<&'static Encoding>) -> (Codec, &'static str) {
        match self {
            Decision::Fixed { codec, by } => (*codec, by),
            Decision::Preference { candidates, otherwise } => match pref.filter(|p| candidates.contains(p)) {
                Some(p) => (Codec::Enc(p), "preference"),
                None => *otherwise,
            },
        }
    }

    /// Every codec a path may have been written with, `first` first.
    pub fn fallbacks(&self, first: Codec) -> Vec<Codec> {
        let mut out = vec![first];
        if let Decision::Preference { candidates, otherwise } = self {
            for c in candidates.iter().map(|e| Codec::Enc(e)).chain([otherwise.0, Codec::Cp437]) {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        out
    }
}

pub fn decide(names: &[RawName]) -> Decision {
    let rest: Vec<&[u8]> = names.iter().filter(|n| n.unicode.is_none()).map(|n| n.raw.as_slice()).collect();
    if rest.iter().all(|r| r.is_ascii()) {
        return Decision::Fixed { codec: Codec::Utf8, by: "flag" };
    }
    if rest.iter().all(|r| std::str::from_utf8(r).is_ok()) {
        return Decision::Fixed { codec: Codec::Utf8, by: "utf8" };
    }
    let mut segments: Vec<&[u8]> = rest.iter().flat_map(|r| r.split(|&b| b == b'/')).filter(|s| !s.is_ascii()).collect();
    segments.sort();
    segments.dedup();
    let n: usize = segments.iter().map(|s| s.iter().filter(|b| !b.is_ascii()).count()).sum();
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Deny);
    let joined = segments.join(&b'\n');
    detector.feed(&joined, true);
    let guess = detector.guess(None, Utf8Detection::Deny);
    let candidates: Vec<&'static Encoding> = CANDIDATES
        .into_iter()
        .filter(|e| rest.iter().all(|r| e.decode_without_bom_handling_and_without_replacement(r).is_some()))
        .collect();
    let detected = candidates.contains(&guess);
    if n >= DETECT_ALONE {
        return if detected {
            Decision::Fixed { codec: Codec::Enc(guess), by: "detected" }
        } else {
            Decision::Fixed { codec: Codec::Cp437, by: "fallback" }
        };
    }
    let otherwise = if n >= DETECT_WITH_HINT && detected { (Codec::Enc(guess), "detected") } else { (Codec::Cp437, "fallback") };
    if candidates.is_empty() {
        return Decision::Fixed { codec: otherwise.0, by: otherwise.1 };
    }
    Decision::Preference { candidates, otherwise }
}

/// Why an entry is not addressable (D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unaddressable {
    Undecodable,
    Separators,
    Unsafe,
    Symlink,
    Shadowed,
}

/// `(segments, raw segments, is_dir)` of one entry, after decoding and the
/// separator rules of step 5 (`backslash`: a lone `\` separates, as Windows
/// tools write it; tar names keep it as a character).
#[allow(clippy::type_complexity)]
pub fn split_entry(name: &RawName, codec: Codec, backslash: bool) -> Result<(Vec<String>, Vec<Vec<u8>>, bool), Unaddressable> {
    let (decoded, raw_codec) = match &name.unicode {
        Some(u) => (u.clone(), Codec::Utf8),
        None => (codec.decode(&name.raw).ok_or(Unaddressable::Undecodable)?, codec),
    };
    let slash = decoded.contains('/');
    let backslash = backslash && decoded.contains('\\');
    if slash && backslash {
        return Err(Unaddressable::Separators);
    }
    let sep = if backslash { b'\\' } else { b'/' };
    let text = if backslash { decoded.replace('\\', "/") } else { decoded };
    let is_dir = text.ends_with('/');
    let body = text.strip_suffix('/').unwrap_or(&text);
    if body.is_empty() || body.starts_with('/') {
        return Err(Unaddressable::Unsafe);
    }
    let segments: Vec<String> = body.split('/').map(String::from).collect();
    if segments.iter().any(|s| validate_name(s).is_err()) {
        return Err(Unaddressable::Unsafe);
    }
    if segments[0].len() == 2 && segments[0].as_bytes()[0].is_ascii_alphabetic() && segments[0].as_bytes()[1] == b':' {
        return Err(Unaddressable::Unsafe);
    }
    let raw = &name.raw;
    let mut cuts = Vec::new();
    for (i, &b) in raw.iter().enumerate() {
        if b == sep && raw_codec.whole_char_at(raw, i) {
            cuts.push(i);
        }
    }
    let mut raw_segments = Vec::with_capacity(segments.len());
    let mut start = 0;
    for cut in cuts.iter().copied().chain([raw.len()]) {
        raw_segments.push(raw[start..cut].to_vec());
        start = cut + 1;
    }
    if is_dir {
        raw_segments.pop();
    }
    if raw_segments.len() != segments.len() {
        return Err(Unaddressable::Undecodable);
    }
    Ok((segments, raw_segments, is_dir))
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Member path (`""` for the archive root).
    pub path: String,
    pub name: String,
    /// Raw segments joined with `/`.
    pub raw: Vec<u8>,
    pub dir: bool,
    /// Central directory entry behind this node (implicit directories have none).
    pub entry: Option<usize>,
    pub children: Vec<usize>,
}

/// The addressable members under one codec.
#[derive(Debug)]
pub struct Tree {
    pub codec: Codec,
    pub by: &'static str,
    pub nodes: Vec<TreeNode>,
    by_path: HashMap<String, usize>,
    by_raw: HashMap<Vec<u8>, usize>,
    ambiguous: HashSet<String>,
    pub hidden: usize,
}

pub const ROOT: usize = 0;

impl Tree {
    /// `entries[i]` is `(name, symlink, readable)`; `readable` is false for
    /// entries the zip crate cannot address on its own (shadowed duplicates).
    pub fn build(entries: &[(&RawName, bool, bool)], codec: Codec, by: &'static str, backslash: bool) -> Tree {
        let mut tree = Tree {
            codec,
            by,
            nodes: vec![TreeNode { path: String::new(), name: String::new(), raw: Vec::new(), dir: true, entry: None, children: Vec::new() }],
            by_path: HashMap::new(),
            by_raw: HashMap::new(),
            ambiguous: HashSet::new(),
            hidden: 0,
        };
        tree.by_path.insert(String::new(), ROOT);
        let mut conflicts: HashSet<usize> = HashSet::new();
        let mut placed: Vec<Option<usize>> = vec![None; entries.len()];
        for (i, (name, symlink, readable)) in entries.iter().enumerate() {
            if *symlink {
                continue;
            }
            let Ok((segments, raw_segments, is_dir)) = split_entry(name, codec, backslash) else { continue };
            let mut parent = ROOT;
            let mut blocked = false;
            for k in 1..segments.len() {
                let path = segments[..k].join("/");
                parent = match tree.by_path.get(&path) {
                    Some(&idx) => {
                        if !tree.nodes[idx].dir {
                            conflicts.insert(idx);
                            blocked = true;
                            break;
                        }
                        idx
                    }
                    None => tree.add(parent, path, segments[k - 1].clone(), raw_segments[..k].join(&b'/'), true, None),
                };
            }
            if blocked {
                continue;
            }
            let path = segments.join("/");
            match tree.by_path.get(&path).copied() {
                Some(idx) if is_dir && tree.nodes[idx].dir => {
                    if tree.nodes[idx].entry.is_none() {
                        tree.nodes[idx].entry = Some(i);
                    }
                    placed[i] = Some(idx);
                }
                Some(idx) => {
                    conflicts.insert(idx);
                }
                None => {
                    let idx = tree.add(parent, path, segments.last().unwrap().clone(), raw_segments.join(&b'/'), is_dir, Some(i));
                    if *readable || is_dir {
                        placed[i] = Some(idx);
                    } else {
                        conflicts.insert(idx);
                    }
                }
            }
        }
        for idx in &conflicts {
            tree.ambiguous.insert(tree.nodes[*idx].path.clone());
        }
        let mut reachable = vec![false; tree.nodes.len()];
        let mut stack = vec![ROOT];
        while let Some(idx) = stack.pop() {
            reachable[idx] = true;
            let kids: Vec<usize> = tree.nodes[idx].children.iter().copied().filter(|c| !conflicts.contains(c)).collect();
            stack.extend(kids.iter().copied());
            tree.nodes[idx].children = kids;
        }
        tree.by_path.retain(|_, idx| reachable[*idx]);
        for (idx, node) in tree.nodes.iter().enumerate() {
            if reachable[idx] {
                tree.by_raw.insert(node.raw.clone(), idx);
            }
        }
        let visible = placed.iter().filter(|p| p.is_some_and(|idx| reachable[idx])).count();
        tree.hidden = entries.len() - visible;
        for idx in 0..tree.nodes.len() {
            let mut kids = std::mem::take(&mut tree.nodes[idx].children);
            kids.sort_by(|a, b| tree.nodes[*a].name.as_bytes().cmp(tree.nodes[*b].name.as_bytes()));
            tree.nodes[idx].children = kids;
        }
        tree
    }

    fn add(&mut self, parent: usize, path: String, name: String, raw: Vec<u8>, dir: bool, entry: Option<usize>) -> usize {
        let idx = self.nodes.len();
        self.by_path.insert(path.clone(), idx);
        self.nodes.push(TreeNode { path, name, raw, dir, entry, children: Vec::new() });
        self.nodes[parent].children.push(idx);
        idx
    }

    pub fn by_path(&self, path: &str) -> Option<usize> {
        self.by_path.get(path).copied()
    }

    pub fn by_raw(&self, raw: &[u8]) -> Option<usize> {
        self.by_raw.get(raw).copied()
    }

    pub fn is_ambiguous(&self, path: &str) -> bool {
        self.ambiguous.contains(path)
    }

    pub fn child(&self, parent: usize, name: &str) -> Option<usize> {
        self.nodes[parent].children.iter().copied().find(|c| self.nodes[*c].name == name)
    }
}

/// The trees of one archive: built once per codec a request needs.
#[derive(Debug)]
pub struct Naming {
    pub decision: Decision,
    names: Vec<RawName>,
    flags: Vec<(bool, bool)>,
    backslash: bool,
    trees: std::sync::Mutex<HashMap<&'static str, Arc<Tree>>>,
}

impl Naming {
    /// `flags[i]` is `(hidden kind, readable)` of entry `i`.
    pub fn new(names: Vec<RawName>, flags: Vec<(bool, bool)>, backslash: bool) -> Naming {
        let decision = decide(&names);
        Naming { decision, names, flags, backslash, trees: Default::default() }
    }

    pub fn tree(&self, codec: Codec, by: &'static str) -> Arc<Tree> {
        let mut trees = self.trees.lock().unwrap();
        trees
            .entry(codec.name())
            .or_insert_with(|| {
                let entries: Vec<(&RawName, bool, bool)> = self.names.iter().zip(&self.flags).map(|(n, (symlink, readable))| (n, *symlink, *readable)).collect();
                Arc::new(Tree::build(&entries, codec, by, self.backslash))
            })
            .clone()
    }

    /// The tree a requester sees, and the others its paths may come from.
    pub fn trees_for(&self, pref: Option<&'static Encoding>) -> Vec<Arc<Tree>> {
        let (codec, by) = self.decision.for_request(pref);
        self.decision
            .fallbacks(codec)
            .into_iter()
            .map(|c| if c == codec { self.tree(c, by) } else { self.tree(c, self.by_of(c)) })
            .collect()
    }

    fn by_of(&self, codec: Codec) -> &'static str {
        match &self.decision {
            Decision::Preference { otherwise, .. } if otherwise.0 == codec => otherwise.1,
            Decision::Preference { .. } if codec == Codec::Cp437 => "fallback",
            _ => "preference",
        }
    }

    /// The name of member `entry` as a requester sees it (download names).
    pub fn display_name(&self, entry: usize, codec: Codec) -> Option<String> {
        let name = self.names.get(entry)?;
        name.unicode.clone().or_else(|| codec.decode(&name.raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(b: &[u8]) -> RawName {
        RawName { raw: b.to_vec(), unicode: None }
    }

    fn enc(e: &'static Encoding, s: &str) -> RawName {
        raw(&e.encode(s).0)
    }

    fn tree_of(names: &[RawName], pref: Option<&'static Encoding>) -> Arc<Tree> {
        let naming = Naming::new(names.to_vec(), vec![(false, true); names.len()], true);
        naming.trees_for(pref).remove(0)
    }

    fn paths(t: &Tree) -> Vec<String> {
        let mut v: Vec<String> = t.nodes.iter().filter(|n| t.by_path(&n.path).is_some() && !n.path.is_empty()).map(|n| n.path.clone()).collect();
        v.sort();
        v
    }

    #[test]
    fn flagged_and_ascii_names_are_utf8() {
        let names = vec![RawName { raw: "文档/a.txt".as_bytes().to_vec(), unicode: Some("文档/a.txt".into()) }, raw(b"b.txt")];
        let t = tree_of(&names, None);
        assert_eq!((t.codec, t.by), (Codec::Utf8, "flag"));
        assert_eq!(paths(&t), vec!["b.txt", "文档", "文档/a.txt"]);
    }

    #[test]
    fn unflagged_utf8_is_utf8() {
        let t = tree_of(&[raw("报告/季度总结.docx".as_bytes())], None);
        assert_eq!((t.codec, t.by), (Codec::Utf8, "utf8"));
        assert_eq!(paths(&t), vec!["报告", "报告/季度总结.docx"]);
    }

    #[test]
    fn long_cjk_names_are_detected() {
        let cases: [(&'static Encoding, &[&str]); 4] = [
            (GBK, &["项目资料/会议纪要.docx", "项目资料/财务报表汇总.xlsx", "照片/春节聚会.jpg"]),
            (BIG5, &["專案資料/會議紀錄.docx", "專案資料/財務報表彙總.xlsx", "照片/春節聚會.jpg"]),
            (SHIFT_JIS, &["プロジェクト資料/会議議事録.docx", "プロジェクト資料/財務報告書.xlsx", "写真/お正月.jpg"]),
            (EUC_KR, &["프로젝트자료/회의록정리.docx", "프로젝트자료/재무보고서.xlsx", "사진/설날모임.jpg"]),
        ];
        for (e, names) in cases {
            let encoded: Vec<RawName> = names.iter().map(|n| enc(e, n)).collect();
            for pref in [None, Some(GBK), Some(SHIFT_JIS)] {
                let t = tree_of(&encoded, pref);
                assert_eq!((t.codec, t.by), (Codec::Enc(e), "detected"), "{} with {:?}", e.name(), pref.map(|p| p.name()));
                assert!(t.by_path(names[0]).is_some());
            }
        }
    }

    #[test]
    fn short_names_follow_the_preference() {
        let names = vec![enc(GBK, "报告.txt")];
        let naming = Naming::new(names.clone(), vec![(false, true)], true);
        assert!(matches!(naming.decision, Decision::Preference { .. }));
        let zh = naming.trees_for(Some(GBK)).remove(0);
        assert_eq!((zh.codec, zh.by), (Codec::Enc(GBK), "preference"));
        assert!(zh.by_path("报告.txt").is_some());
        let none = naming.trees_for(None);
        assert_eq!(none[0].by, "fallback");
        assert!(none.iter().any(|t| t.by_path("报告.txt").is_some()), "a GBK path still resolves for other requesters");
        let ja = naming.trees_for(Some(SHIFT_JIS));
        assert!(ja.iter().any(|t| t.by_path("报告.txt").is_some()));
    }

    #[test]
    fn western_cp437_names_fall_back() {
        let names = vec![raw(b"M\x81ller.docx"), raw(b"Caf\x82/men\x81.txt")];
        let t = tree_of(&names, None);
        assert_eq!(t.codec.name(), "IBM437");
        assert_eq!(paths(&t), vec!["Café", "Café/menü.txt", "Müller.docx"]);
    }

    #[test]
    fn backslash_separators_and_cp932_trail_bytes() {
        let t = tree_of(&[raw(b"docs\\a.txt"), raw(b"docs\\sub\\b.txt")], None);
        assert_eq!(paths(&t), vec!["docs", "docs/a.txt", "docs/sub", "docs/sub/b.txt"]);
        let names: Vec<RawName> = ["表計算/ソフト一覧表.xlsx", "表計算/十年分の予定表.txt", "表/ソ/十.txt", "資料/能力表.docx"].iter().map(|n| enc(SHIFT_JIS, n)).collect();
        assert!(names[0].raw.contains(&0x5C));
        let t = tree_of(&names, Some(SHIFT_JIS));
        assert_eq!(t.codec, Codec::Enc(SHIFT_JIS));
        assert!(t.by_path("表計算/ソフト一覧表.xlsx").is_some());
        assert!(t.by_path("表/ソ/十.txt").is_some());
        let node = &t.nodes[t.by_path("表計算").unwrap()];
        assert_eq!(node.raw, SHIFT_JIS.encode("表計算").0.to_vec());
        let mixed = tree_of(&[raw(b"a/b\\c.txt")], None);
        assert_eq!(mixed.hidden, 1);
    }

    #[test]
    fn unsafe_duplicate_and_symlink_entries_are_hidden() {
        let names = vec![raw(b"../evil.txt"), raw(b"/abs.txt"), raw(b"a/./b"), raw(b"ok.txt"), raw(b"dup.txt"), raw(b"dup.txt"), raw(b"link"), raw(b"C:/x.txt"), raw(b"d/"), raw(b"d/x")];
        let mut flags = vec![(false, true); names.len()];
        flags[6].0 = true;
        let naming = Naming::new(names, flags, true);
        let t = naming.trees_for(None).remove(0);
        assert_eq!(paths(&t), vec!["d", "d/x", "ok.txt"]);
        assert!(t.is_ambiguous("dup.txt"));
        assert_eq!(t.hidden, 7);
    }

    #[test]
    fn file_and_directory_with_one_name_are_ambiguous() {
        let t = tree_of(&[raw(b"a"), raw(b"a/b.txt"), raw(b"c.txt")], None);
        assert!(t.is_ambiguous("a"));
        assert_eq!(paths(&t), vec!["c.txt"]);
        assert_eq!(t.hidden, 2, "the file and the entry below the clashing directory");
    }

    #[test]
    fn implicit_directories_and_named_at_members() {
        let t = tree_of(&[raw(b"x/y/z.txt"), raw(b"@/content/a"), raw(b"x/")], None);
        assert_eq!(paths(&t), vec!["@", "@/content", "@/content/a", "x", "x/y", "x/y/z.txt"]);
        assert_eq!(t.hidden, 0);
        let x = t.by_path("x").unwrap();
        assert_eq!(t.nodes[x].entry, Some(2));
        assert_eq!(t.by_raw(b"x/y"), t.by_path("x/y"));
    }

    #[test]
    fn accept_language_preference() {
        assert_eq!(preference("en-US,zh-CN;q=0.8"), Some(GBK));
        assert_eq!(preference("zh-Hant-TW"), Some(BIG5));
        assert_eq!(preference("zh-HK"), Some(BIG5));
        assert_eq!(preference("ja-JP"), Some(SHIFT_JIS));
        assert_eq!(preference("ko"), Some(EUC_KR));
        assert_eq!(preference("en-US,fr"), None);
    }
}
