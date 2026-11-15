//! The screen's `input` messages as DevTools input commands (Agent Browser
//! Protocol §3.2). Coordinates are CSS pixels of the viewport on both sides.

use serde_json::{json, Value};

/// A touch point that is down: `(id, x, y)`.
pub type Touch = (i64, f64, f64);

fn number(message: &Value, name: &str) -> Result<f64, String> {
    message[name].as_f64().filter(|n| n.is_finite()).ok_or_else(|| format!("input `{name}` is not a number"))
}

fn modifiers(message: &Value) -> i64 {
    message["modifiers"].as_i64().unwrap_or(0) & 0xf
}

fn points(touches: &[Touch]) -> Vec<Value> {
    touches.iter().map(|(id, x, y)| json!({ "id": id, "x": x, "y": y })).collect()
}

/// The commands of one `input` message, in order. `touches` are the points
/// that are down, as the browser was last told.
pub fn commands(message: &Value, touches: &mut Vec<Touch>) -> Result<Vec<(&'static str, Value)>, String> {
    match message["kind"].as_str() {
        Some("touch") => touch(message, touches),
        Some("mouse") => mouse(message).map(|command| vec![command]),
        Some("wheel") => Ok(vec![(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mouseWheel",
                "x": number(message, "x")?,
                "y": number(message, "y")?,
                "deltaX": message["delta_x"].as_f64().unwrap_or(0.0),
                "deltaY": message["delta_y"].as_f64().unwrap_or(0.0),
                "modifiers": modifiers(message),
            }),
        )]),
        Some("key") => key(message).map(|command| vec![command]),
        Some("text") => {
            let text = message["text"].as_str().ok_or("input `text` is not a string")?;
            Ok(if text.is_empty() { Vec::new() } else { vec![("Input.insertText", json!({ "text": text }))] })
        }
        Some("composition") => {
            let text = message["text"].as_str().ok_or("input `text` is not a string")?;
            let end = text.chars().count() as i64;
            let caret = |name: &str| message[name].as_i64().unwrap_or(end).clamp(0, end);
            Ok(vec![(
                "Input.imeSetComposition",
                json!({ "text": text, "selectionStart": caret("selection_start"), "selectionEnd": caret("selection_end") }),
            )])
        }
        other => Err(format!("unknown input kind {other:?}")),
    }
}

/// `points` are the touch points down after the event. The browser is told
/// the other way round at a release: which points went up (none named: all).
fn touch(message: &Value, touches: &mut Vec<Touch>) -> Result<Vec<(&'static str, Value)>, String> {
    let mut now = Vec::new();
    for point in message["points"].as_array().into_iter().flatten() {
        let id = point["id"].as_i64().ok_or("a touch point has no id")?;
        now.push((id, number(point, "x")?, number(point, "y")?));
    }
    let event = |kind: &str, touch_points: Vec<Value>| ("Input.dispatchTouchEvent", json!({ "type": kind, "touchPoints": touch_points }));
    Ok(match message["phase"].as_str() {
        Some("start" | "move") if now.is_empty() => return Err("a touch start or move names no point".into()),
        Some("start") => {
            *touches = now;
            vec![event("touchStart", points(touches))]
        }
        Some("move") => {
            // A point the browser has not seen go down is a start.
            let kind = if now.iter().all(|(id, ..)| touches.iter().any(|(known, ..)| known == id)) { "touchMove" } else { "touchStart" };
            *touches = now;
            vec![event(kind, points(touches))]
        }
        Some("end") => {
            let lifted: Vec<Touch> = touches.iter().filter(|(id, ..)| !now.iter().any(|(still, ..)| still == id)).cloned().collect();
            let was_down = !touches.is_empty();
            *touches = now;
            match (touches.is_empty(), lifted.is_empty()) {
                (true, _) if was_down => vec![event("touchEnd", Vec::new())],
                (false, false) => vec![event("touchEnd", points(&lifted))],
                _ => Vec::new(),
            }
        }
        Some("cancel") => {
            let was_down = !touches.is_empty();
            touches.clear();
            if was_down { vec![event("touchCancel", Vec::new())] } else { Vec::new() }
        }
        other => return Err(format!("unknown touch phase {other:?}")),
    })
}

fn mouse(message: &Value) -> Result<(&'static str, Value), String> {
    let button = match message["button"].as_str().unwrap_or("none") {
        button @ ("left" | "middle" | "right" | "none") => button,
        other => return Err(format!("unknown mouse button `{other}`")),
    };
    let held = match button {
        "left" => 1,
        "right" => 2,
        "middle" => 4,
        _ => 0,
    };
    let (kind, buttons, clicks) = match message["action"].as_str() {
        Some("down") => ("mousePressed", held, message["click_count"].as_i64().unwrap_or(1).clamp(1, 3)),
        Some("up") => ("mouseReleased", 0, message["click_count"].as_i64().unwrap_or(1).clamp(1, 3)),
        Some("move") => ("mouseMoved", held, 0),
        other => return Err(format!("unknown mouse action {other:?}")),
    };
    Ok((
        "Input.dispatchMouseEvent",
        json!({
            "type": kind,
            "x": number(message, "x")?,
            "y": number(message, "y")?,
            "button": button,
            "buttons": buttons,
            "clickCount": clicks,
            "modifiers": modifiers(message),
        }),
    ))
}

/// `(windowsVirtualKeyCode, location)` of a DOM `key` / `code` pair
/// (location: 1 left, 2 right, 3 numpad).
pub fn virtual_key(key: &str, code: &str) -> (i64, i64) {
    let location = if code.starts_with("Numpad") {
        3
    } else if code.ends_with("Left") && !code.starts_with("Arrow") && !code.starts_with("Bracket") {
        1
    } else if code.ends_with("Right") && !code.starts_with("Arrow") && !code.starts_with("Bracket") {
        2
    } else {
        0
    };
    let by_code = match code {
        c if c.len() == 4 && c.starts_with("Key") => c.as_bytes()[3] as i64,
        c if c.len() == 6 && c.starts_with("Digit") => c.as_bytes()[5] as i64,
        c if c.len() == 7 && c.starts_with("Numpad") && c.as_bytes()[6].is_ascii_digit() => 96 + (c.as_bytes()[6] - b'0') as i64,
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        _ => 0,
    };
    let by_key = match key {
        "Enter" => 13,
        "Backspace" => 8,
        "Tab" => 9,
        "Escape" => 27,
        " " => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "Shift" => 16,
        "Control" => 17,
        "Alt" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Meta" => 91,
        "ContextMenu" => 93,
        "NumLock" => 144,
        "ScrollLock" => 145,
        k if k.len() >= 2 && k.starts_with('F') && k[1..].parse::<i64>().is_ok_and(|n| (1..=24).contains(&n)) => 111 + k[1..].parse::<i64>().unwrap_or(0),
        _ => 0,
    };
    // A named key is what it is wherever it sits (Enter of the numpad); a
    // character is the key it was typed with, else the character itself
    // (a soft keyboard has no `code`).
    let vk = match (by_key, by_code) {
        (0, 0) => match key.as_bytes() {
            [c] if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as i64,
            _ => 0,
        },
        (0, by_code) => by_code,
        (by_key, _) => by_key,
    };
    (vk, location)
}

fn key(message: &Value) -> Result<(&'static str, Value), String> {
    let key = message["key"].as_str().unwrap_or_default();
    let code = message["code"].as_str().unwrap_or_default();
    if key.is_empty() && code.is_empty() {
        return Err("a key input names no key".into());
    }
    let (vk, location) = virtual_key(key, code);
    let mut params = json!({
        "key": key,
        "code": code,
        "modifiers": modifiers(message),
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk,
        "location": location,
        "isKeypad": location == 3,
    });
    match message["action"].as_str() {
        Some("down") => {
            // Enter types a line break (and submits a form) only with its character.
            let text = match (message["text"].as_str().unwrap_or_default(), key) {
                ("", "Enter") => "\r",
                (text, _) => text,
            };
            if text.is_empty() {
                params["type"] = json!("rawKeyDown");
            } else {
                params["type"] = json!("keyDown");
                params["text"] = json!(text);
                params["unmodifiedText"] = json!(text);
            }
        }
        Some("up") => params["type"] = json!("keyUp"),
        other => return Err(format!("unknown key action {other:?}")),
    }
    Ok(("Input.dispatchKeyEvent", params))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(message: Value, touches: &mut Vec<Touch>) -> (&'static str, Value) {
        let mut all = commands(&message, touches).unwrap();
        assert_eq!(all.len(), 1, "{message}");
        all.remove(0)
    }

    #[test]
    fn touches_tell_the_browser_which_points_went_up() {
        let mut down = Vec::new();
        let point = |id: i64, x: f64| json!({ "id": id, "x": x, "y": 300.0 });
        let touch = |phase: &str, points: Vec<Value>| json!({ "kind": "touch", "phase": phase, "points": points });
        let (method, start) = one(touch("start", vec![point(0, 10.0)]), &mut down);
        assert_eq!((method, start["type"].as_str()), ("Input.dispatchTouchEvent", Some("touchStart")));
        assert_eq!(start["touchPoints"], json!([{ "id": 0, "x": 10.0, "y": 300.0 }]));
        // A second finger: every point that is down is named.
        let (_, second) = one(touch("start", vec![point(0, 10.0), point(1, 90.0)]), &mut down);
        assert_eq!(second["touchPoints"].as_array().unwrap().len(), 2);
        let (_, moved) = one(touch("move", vec![point(0, 12.0), point(1, 95.0)]), &mut down);
        assert_eq!((moved["type"].as_str(), moved["touchPoints"][1]["x"].as_f64()), (Some("touchMove"), Some(95.0)));
        // One finger goes up: `points` are those still down, the browser is told the one that left.
        let (_, lifted) = one(touch("end", vec![point(0, 12.0)]), &mut down);
        assert_eq!(lifted, json!({ "type": "touchEnd", "touchPoints": [{ "id": 1, "x": 95.0, "y": 300.0 }] }));
        assert_eq!(down, vec![(0, 12.0, 300.0)]);
        let (_, last) = one(touch("end", vec![]), &mut down);
        assert_eq!(last, json!({ "type": "touchEnd", "touchPoints": [] }));
        assert!(down.is_empty());
        // Nothing is down: an end or a cancel says nothing.
        assert!(commands(&touch("end", vec![]), &mut down).unwrap().is_empty());
        assert!(commands(&touch("cancel", vec![]), &mut down).unwrap().is_empty());
        one(touch("start", vec![point(3, 1.0)]), &mut down);
        let (_, cancelled) = one(touch("cancel", vec![]), &mut down);
        assert_eq!(cancelled, json!({ "type": "touchCancel", "touchPoints": [] }));
        // A move of a point that never went down starts it.
        let (_, late) = one(touch("move", vec![point(7, 1.0)]), &mut down);
        assert_eq!(late["type"], "touchStart");
        assert!(commands(&touch("start", vec![]), &mut down).is_err());
        assert!(commands(&touch("swipe", vec![point(0, 1.0)]), &mut down).is_err());
        assert!(commands(&json!({ "kind": "touch", "phase": "start", "points": [{ "id": 0, "x": "1", "y": 2 }] }), &mut down).is_err());
    }

    #[test]
    fn mouse_and_wheel_keep_their_coordinates_and_buttons() {
        let mut down = Vec::new();
        let mouse = |action: &str, button: &str| json!({ "kind": "mouse", "action": action, "x": 10.5, "y": 20, "button": button, "click_count": 2, "modifiers": 8 });
        let (method, pressed) = one(mouse("down", "left"), &mut down);
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(pressed, json!({ "type": "mousePressed", "x": 10.5, "y": 20.0, "button": "left", "buttons": 1, "clickCount": 2, "modifiers": 8 }));
        let (_, released) = one(mouse("up", "left"), &mut down);
        assert_eq!((released["type"].as_str(), released["buttons"].as_i64()), (Some("mouseReleased"), Some(0)));
        let (_, dragged) = one(mouse("move", "right"), &mut down);
        assert_eq!((dragged["type"].as_str(), dragged["buttons"].as_i64(), dragged["clickCount"].as_i64()), (Some("mouseMoved"), Some(2), Some(0)));
        let (_, hovered) = one(json!({ "kind": "mouse", "action": "move", "x": 1, "y": 2 }), &mut down);
        assert_eq!((hovered["button"].as_str(), hovered["buttons"].as_i64()), (Some("none"), Some(0)));
        assert!(commands(&mouse("down", "fourth"), &mut down).is_err());
        assert!(commands(&json!({ "kind": "mouse", "action": "down", "x": 1 }), &mut down).is_err());
        let (_, wheel) = one(json!({ "kind": "wheel", "x": 5, "y": 6, "delta_x": 0, "delta_y": 120, "modifiers": 2 }), &mut down);
        assert_eq!(wheel, json!({ "type": "mouseWheel", "x": 5.0, "y": 6.0, "deltaX": 0.0, "deltaY": 120.0, "modifiers": 2 }));
    }

    #[test]
    fn keys_carry_their_virtual_key_codes() {
        for (key, code, vk, location) in [
            ("a", "KeyA", 65, 0),
            ("A", "KeyA", 65, 0),
            ("z", "KeyZ", 90, 0),
            ("7", "Digit7", 55, 0),
            ("&", "Digit7", 55, 0),
            ("7", "Numpad7", 103, 3),
            ("Enter", "Enter", 13, 0),
            ("Enter", "NumpadEnter", 13, 3),
            ("Backspace", "Backspace", 8, 0),
            ("Tab", "Tab", 9, 0),
            ("Escape", "Escape", 27, 0),
            ("ArrowLeft", "ArrowLeft", 37, 0),
            ("ArrowUp", "ArrowUp", 38, 0),
            ("ArrowRight", "ArrowRight", 39, 0),
            ("ArrowDown", "ArrowDown", 40, 0),
            ("Delete", "Delete", 46, 0),
            ("Home", "Home", 36, 0),
            ("End", "End", 35, 0),
            ("PageUp", "PageUp", 33, 0),
            ("PageDown", "PageDown", 34, 0),
            (" ", "Space", 32, 0),
            ("Shift", "ShiftLeft", 16, 1),
            ("Control", "ControlRight", 17, 2),
            ("Alt", "AltLeft", 18, 1),
            ("Meta", "MetaLeft", 91, 1),
            ("F5", "F5", 116, 0),
            ("F12", "F12", 123, 0),
            (";", "Semicolon", 186, 0),
            ("/", "Slash", 191, 0),
            ("[", "BracketLeft", 219, 0),
            ("]", "BracketRight", 221, 0),
            // A soft keyboard names no physical key.
            ("q", "", 81, 0),
            ("5", "", 53, 0),
            ("中", "", 0, 0),
        ] {
            assert_eq!(virtual_key(key, code), (vk, location), "{key:?} {code:?}");
        }
        let mut down = Vec::new();
        let key = |action: &str, key: &str, code: &str, modifiers: i64, text: &str| json!({ "kind": "key", "action": action, "key": key, "code": code, "modifiers": modifiers, "text": text });
        // A character is typed by the key press that carries it.
        let (method, typed) = one(key("down", "a", "KeyA", 0, "a"), &mut down);
        assert_eq!(method, "Input.dispatchKeyEvent");
        assert_eq!((typed["type"].as_str(), typed["text"].as_str(), typed["windowsVirtualKeyCode"].as_i64()), (Some("keyDown"), Some("a"), Some(65)));
        // A shortcut types nothing: Ctrl+A selects.
        let (_, shortcut) = one(key("down", "a", "KeyA", 2, ""), &mut down);
        assert_eq!((shortcut["type"].as_str(), shortcut["modifiers"].as_i64(), shortcut.get("text")), (Some("rawKeyDown"), Some(2), None));
        let (_, enter) = one(key("down", "Enter", "Enter", 0, ""), &mut down);
        assert_eq!((enter["type"].as_str(), enter["text"].as_str()), (Some("keyDown"), Some("\r")));
        let (_, arrow) = one(key("down", "ArrowDown", "ArrowDown", 8, ""), &mut down);
        assert_eq!((arrow["type"].as_str(), arrow["windowsVirtualKeyCode"].as_i64(), arrow["modifiers"].as_i64()), (Some("rawKeyDown"), Some(40), Some(8)));
        let (_, up) = one(key("up", "a", "KeyA", 0, ""), &mut down);
        assert_eq!(up["type"], "keyUp");
        assert!(commands(&key("press", "a", "KeyA", 0, ""), &mut down).is_err());
        assert!(commands(&key("down", "", "", 0, ""), &mut down).is_err());
    }

    #[test]
    fn text_and_composition_go_to_the_input_method() {
        let mut down = Vec::new();
        assert_eq!(one(json!({ "kind": "text", "text": "整段文字" }), &mut down), ("Input.insertText", json!({ "text": "整段文字" })));
        assert!(commands(&json!({ "kind": "text", "text": "" }), &mut down).unwrap().is_empty());
        assert_eq!(
            one(json!({ "kind": "composition", "text": "zhong", "selection_start": 5, "selection_end": 5 }), &mut down),
            ("Input.imeSetComposition", json!({ "text": "zhong", "selectionStart": 5, "selectionEnd": 5 }))
        );
        // No text: the composition is cancelled. A caret outside the text is put at its end.
        assert_eq!(
            one(json!({ "kind": "composition", "text": "", "selection_start": 3, "selection_end": 3 }), &mut down),
            ("Input.imeSetComposition", json!({ "text": "", "selectionStart": 0, "selectionEnd": 0 }))
        );
        assert!(commands(&json!({ "kind": "scroll" }), &mut down).is_err());
        assert!(commands(&json!({ "kind": "text" }), &mut down).is_err());
    }
}
