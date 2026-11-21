use name_lib::ObjectProfile;

/// Effects accepted by the calendar and to-do action contracts.
pub fn action_effect<'a>(profile: &'a ObjectProfile, action: &str) -> Option<&'a str> {
    let effect = profile
        .actions
        .get(action)?
        .buckyos_action
        .as_ref()?
        .effect
        .as_deref()?;
    match (action, effect) {
        ("list" | "get_submission", "read" | "pure")
        | ("rm", "write" | "destructive")
        | ("add" | "update" | "skip" | "postpone" | "done" | "undone" | "move", "write") => {
            Some(effect)
        }
        _ => None,
    }
}

pub fn matches_slot(profile: &ObjectProfile, slot: &str) -> bool {
    (profile.extra.get("x-buckyos:slot").and_then(|v| v.as_str()) == Some(slot)
        || profile.traits.contains(&format!("buckyos.{slot}")))
        && action_effect(profile, "list").is_some()
}
