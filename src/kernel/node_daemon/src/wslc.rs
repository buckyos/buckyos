use serde_json::Value;
use std::path::PathBuf;

pub(crate) fn program() -> Result<Option<PathBuf>, String> {
    // A product without containers never looks for one.
    if !crate::product::containers_enabled() {
        return Ok(None);
    }
    match std::env::var("BUCKYOS_CONTAINER_RUNTIME").as_deref() {
        Ok("docker") => return Ok(None),
        Ok("wslc") => return Ok(Some(find_program().unwrap_or_else(|| "wslc.exe".into()))),
        Ok(value) => return Err(format!("unknown BUCKYOS_CONTAINER_RUNTIME: {value}")),
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("BUCKYOS_CONTAINER_RUNTIME is not valid Unicode".into());
        }
        Err(std::env::VarError::NotPresent) => {}
    }
    Ok(if cfg!(windows) { find_program() } else { None })
}

fn find_program() -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|path| path.join("wslc.exe"))
        .chain(
            std::env::var_os("ProgramFiles").map(|path| PathBuf::from(path).join("WSL/wslc.exe")),
        )
        .find(|path| path.is_file())
}

#[derive(Debug, PartialEq)]
pub(crate) enum Projection {
    Raw,
    Object,
    Field(&'static str),
    Names,
    VolumeNames,
}

pub(crate) fn command(args: &[String]) -> Result<(Vec<String>, Projection), String> {
    if args.first().map(String::as_str) == Some("images") && args.len() == 3 && args[1] == "-q" {
        return Ok((
            vec![
                "images".into(),
                "-q".into(),
                "--filter".into(),
                format!("reference={}", args[2]),
            ],
            Projection::Raw,
        ));
    }
    if !matches!(
        args.first().map(String::as_str),
        Some("container" | "image" | "volume")
    ) {
        return Ok((args.to_vec(), Projection::Raw));
    }
    let mut translated = Vec::new();
    let mut projection = Projection::Raw;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let format = if arg == "--format" {
            index += 1;
            Some(args.get(index).ok_or("missing --format value")?.as_str())
        } else {
            arg.strip_prefix("--format=")
        };
        if let Some(format) = format {
            projection = match format {
                "{{json .}}" => Projection::Object,
                "{{.Id}}" => Projection::Field("Id"),
                "{{json .RepoDigests}}" => Projection::Field("RepoDigests"),
                "{{.Names}}" => Projection::Names,
                "{{.Name}}" => Projection::VolumeNames,
                _ => return Err(format!("unsupported WSLC output format: {format}")),
            };
            translated.extend(["--format".into(), "json".into()]);
        } else {
            translated.push(arg.clone());
        }
        index += 1;
    }
    Ok((translated, projection))
}

pub(crate) fn project(raw: &str, projection: &Projection) -> Result<String, String> {
    if *projection == Projection::Raw {
        return Ok(raw.to_string());
    }
    let mut objects = Vec::new();
    for value in serde_json::Deserializer::from_str(raw).into_iter::<Value>() {
        match value.map_err(|error| format!("invalid WSLC JSON: {error}"))? {
            Value::Array(values) => objects.extend(values),
            value => objects.push(value),
        }
    }
    if matches!(projection, Projection::Names | Projection::VolumeNames) {
        let field = if *projection == Projection::Names {
            "Names"
        } else {
            "Name"
        };
        return objects
            .iter()
            .map(|object| {
                object
                    .get(field)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| format!("WSLC list is missing {field}"))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|names| names.join("\n"));
    }
    if objects.len() != 1 {
        return Err(format!(
            "expected one WSLC inspect object, got {}",
            objects.len()
        ));
    }
    let object = &objects[0];
    match projection {
        Projection::Object => Ok(object.to_string()),
        Projection::Field(field) => {
            let value = object
                .get(field)
                .ok_or_else(|| format!("WSLC inspect is missing {field}"))?;
            Ok(value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string()))
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn translates_image_reference_filter() {
        assert_eq!(
            command(&args(&["images", "-q", "paios/aios:latest-amd64"])).unwrap(),
            (
                args(&[
                    "images",
                    "-q",
                    "--filter",
                    "reference=paios/aios:latest-amd64"
                ]),
                Projection::Raw
            )
        );
    }

    #[test]
    fn projects_inspect_array_and_ndjson_names() {
        let (command, projection) = command(&args(&[
            "container",
            "inspect",
            "--format={{json .}}",
            "app",
        ]))
        .unwrap();
        assert_eq!(
            command,
            args(&["container", "inspect", "--format", "json", "app"])
        );
        assert_eq!(project(r#"[{"Id":"abc","State":{"Running":true},"Config":{"Labels":{"buckyos.app_id":"app"}}}]"#, &projection).unwrap(),
            r#"{"Config":{"Labels":{"buckyos.app_id":"app"}},"Id":"abc","State":{"Running":true}}"#);
        assert_eq!(
            project(
                "{\"Names\":\"one\"}\n{\"Names\":\"two\"}\n",
                &Projection::Names
            )
            .unwrap(),
            "one\ntwo"
        );
        assert_eq!(project("[]", &Projection::Names).unwrap(), "");
    }

    #[test]
    fn projects_image_identity_without_changing_digest() {
        assert_eq!(
            project(
                r#"[{"Id":"sha256:abc","RepoDigests":["image@sha256:def"]}]"#,
                &Projection::Field("Id")
            )
            .unwrap(),
            "sha256:abc"
        );
        assert_eq!(
            project(
                r#"[{"RepoDigests":["image@sha256:def"]}]"#,
                &Projection::Field("RepoDigests")
            )
            .unwrap(),
            r#"["image@sha256:def"]"#
        );
        assert!(project("[]", &Projection::Object).is_err());
        assert!(project("not json", &Projection::Names).is_err());
        assert!(project("[{}]", &Projection::Field("Id")).is_err());
    }
}
