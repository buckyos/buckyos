use serde_json::Value as Json;

pub fn find_string_pointer<'a>(json: &'a Json, pointers: &[&str]) -> Option<&'a str> {
    pointers.iter().find_map(|pointer| {
        json.pointer(pointer)
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })
}
