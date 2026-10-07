//! The BlockTree as the model needs it (许愿格 §5.1, §5.2): every readable Block with its absolute
//! rectangle, Surface, group and frame, what data it shows, and — relative to the wish's Block —
//! direction, distance band and reading order, so "the table on the left" needs no arithmetic.

use aiworkspace_core::model::*;
use aiworkspace_core::value::reference_entity_id;
use aiworkspace_core::WsResult;
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn from(v: Option<&Value>) -> Rect {
        let g = |k: &str| v.and_then(|p| p.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
        Rect { x: g("x"), y: g("y"), w: g("w"), h: g("h") }
    }
    pub fn cx(&self) -> f64 {
        self.x + self.w / 2.0
    }
    pub fn cy(&self) -> f64 {
        self.y + self.h / 2.0
    }
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
    /// Gap between the two rectangles (0 when they touch or overlap).
    pub fn gap(&self, o: &Rect) -> f64 {
        let dx = (o.x - (self.x + self.w)).max(self.x - (o.x + o.w)).max(0.0);
        let dy = (o.y - (self.y + self.h)).max(self.y - (o.y + o.h)).max(0.0);
        dx.max(dy)
    }
}

#[derive(Debug, Clone)]
pub struct Block {
    pub entity_id: String,
    pub surface_id: String,
    pub parent_id: String,
    /// Nearest enclosing group (result groups and user groups), if any.
    pub group_id: Option<String>,
    /// A frame Block whose area holds this Block's centre.
    pub frame_id: Option<String>,
    pub rect: Rect,
    pub renderer: String,
    pub title: Option<String>,
    pub source_id: Option<String>,
    pub payload: JsonMap,
    pub pure_ui: bool,
}

#[derive(Debug, Clone)]
pub struct Group {
    pub entity_id: String,
    pub surface_id: String,
    pub title: Option<String>,
    pub rect: Rect,
}

#[derive(Debug, Clone)]
pub struct Surface {
    pub entity_id: String,
    pub title: String,
    pub mode: String,
}

#[derive(Debug, Clone, Default)]
pub struct Canvas {
    pub surfaces: Vec<Surface>,
    pub groups: Vec<Group>,
    pub blocks: Vec<Block>,
}

const PURE_UI: &[&str] = &["frame", "shape"];

fn walk(ctx: &dyn ReadCtx, can_read: &dyn Fn(&EntityRow) -> WsResult<bool>, parent: &str, surface: &str, origin: (f64, f64), group: Option<&str>, out: &mut Canvas) -> WsResult<()> {
    for edge in ctx.children(parent)? {
        let Some(e) = ctx.entity(&edge.child_id)?.filter(|e| e.alive()) else { continue };
        if !can_read(&e)? {
            continue;
        }
        let mut rect = Rect::from(edge.placement.as_ref());
        rect.x += origin.0;
        rect.y += origin.1;
        match e.type_id.as_str() {
            TYPE_CONTAINER => {
                out.groups.push(Group { entity_id: e.entity_id.clone(), surface_id: surface.into(), title: e.payload.get("title").and_then(Value::as_str).map(str::to_string), rect });
                walk(ctx, can_read, &e.entity_id, surface, (rect.x, rect.y), Some(&e.entity_id), out)?;
            }
            TYPE_CELL => {
                let renderer = e.payload.get("view").and_then(|v| v["type"].as_str()).unwrap_or("").to_string();
                out.blocks.push(Block {
                    entity_id: e.entity_id.clone(),
                    surface_id: surface.into(),
                    parent_id: parent.into(),
                    group_id: group.map(str::to_string),
                    frame_id: None,
                    rect,
                    pure_ui: PURE_UI.contains(&renderer.as_str()),
                    renderer,
                    title: e.payload.get("title").and_then(Value::as_str).map(str::to_string),
                    source_id: e.payload.get("source_ref").and_then(reference_entity_id).map(str::to_string),
                    payload: e.payload.clone(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

impl Canvas {
    pub fn build(ctx: &dyn ReadCtx, can_read: &dyn Fn(&EntityRow) -> WsResult<bool>) -> WsResult<Canvas> {
        let mut c = Canvas::default();
        for edge in ctx.children(SURFACES_ID)? {
            let Some(s) = ctx.entity(&edge.child_id)?.filter(|e| e.alive()) else { continue };
            if !can_read(&s)? {
                continue;
            }
            c.surfaces.push(Surface {
                entity_id: s.entity_id.clone(),
                title: s.payload.get("title").and_then(Value::as_str).or(s.name.as_deref()).unwrap_or(&s.entity_id).to_string(),
                mode: s.payload.get("layout").and_then(|l| l["mode"].as_str()).unwrap_or("free").to_string(),
            });
            walk(ctx, can_read, &s.entity_id, &s.entity_id, (0.0, 0.0), None, &mut c)?;
        }
        // frames: the smallest frame on the same Surface holding a Block's centre
        let frames: Vec<(String, String, Rect)> =
            c.blocks.iter().filter(|b| b.renderer == "frame").map(|b| (b.entity_id.clone(), b.surface_id.clone(), b.rect)).collect();
        for b in &mut c.blocks {
            if b.renderer == "frame" {
                continue;
            }
            b.frame_id = frames
                .iter()
                .filter(|(_, s, r)| *s == b.surface_id && r.contains_point(b.rect.cx(), b.rect.cy()))
                .min_by(|a, b2| (a.2.w * a.2.h).partial_cmp(&(b2.2.w * b2.2.h)).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(id, _, _)| id.clone());
        }
        Ok(c)
    }

    pub fn block(&self, id: &str) -> Option<&Block> {
        self.blocks.iter().find(|b| b.entity_id == id)
    }

    pub fn group(&self, id: &str) -> Option<&Group> {
        self.groups.iter().find(|g| g.entity_id == id)
    }

    pub fn surface(&self, id: &str) -> Option<&Surface> {
        self.surfaces.iter().find(|s| s.entity_id == id)
    }

    /// Blocks showing `entity_id`.
    pub fn showing(&self, entity_id: &str) -> Vec<&Block> {
        self.blocks.iter().filter(|b| b.source_id.as_deref() == Some(entity_id)).collect()
    }
}

/// Where `b` is seen from `origin`: (direction, distance band, gap in px).
pub fn relation(origin: &Rect, b: &Rect) -> (&'static str, &'static str, f64) {
    let gap = origin.gap(b);
    let overlap_x = b.x < origin.x + origin.w && b.x + b.w > origin.x;
    let overlap_y = b.y < origin.y + origin.h && b.y + b.h > origin.y;
    let (dx, dy) = (b.cx() - origin.cx(), b.cy() - origin.cy());
    let dir = if overlap_x && overlap_y {
        "重叠"
    } else if overlap_y || (!overlap_x && dx.abs() >= dy.abs()) {
        if dx < 0.0 { "左侧" } else { "右侧" }
    } else if dy < 0.0 {
        "上方"
    } else {
        "下方"
    };
    let band = if gap <= 80.0 {
        "相邻"
    } else if gap <= 600.0 {
        "较近"
    } else {
        "较远"
    };
    (dir, band, gap)
}

/// Reading order on one Surface: rows of Blocks whose tops are within 60 px, left to right.
pub fn reading_order(blocks: &[&Block]) -> Vec<String> {
    let mut v: Vec<&Block> = blocks.to_vec();
    v.sort_by(|a, b| a.rect.y.partial_cmp(&b.rect.y).unwrap_or(std::cmp::Ordering::Equal));
    let mut rows: Vec<Vec<&Block>> = Vec::new();
    for b in v {
        match rows.last_mut() {
            Some(r) if (b.rect.y - r[0].rect.y).abs() <= 60.0 => r.push(b),
            _ => rows.push(vec![b]),
        }
    }
    let mut out = Vec::new();
    for mut r in rows {
        r.sort_by(|a, b| a.rect.x.partial_cmp(&b.rect.x).unwrap_or(std::cmp::Ordering::Equal));
        out.extend(r.into_iter().map(|b| b.entity_id.clone()));
    }
    out
}
