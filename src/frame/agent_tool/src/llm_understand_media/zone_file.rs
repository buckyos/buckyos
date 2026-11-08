//! System URLs and Zone paths as media: the ways the system names a file
//! besides an object id (doc/key url.md) — `cyfs:///<path>` and its common
//! misspellings, `cyfs://<objid>/`, `file://`, `obj://dfs/<path>`, and the path
//! Files shows (`/home/<owner>/…`).
//!
//! A Zone file is read where this runtime mounts it: an Agent's container
//! mounts its owner's home at the host's path (`$BUCKYOS_ROOT/data/home/<owner>`),
//! a host runtime sees the whole data root. A Zone file not mounted here is
//! refused, naming what was looked at.

use std::path::{Component, Path, PathBuf};

use buckyos_api::{ObjHost, ObjectUrl, ResourceRef};

use super::MediaSource;

/// `raw` as media when it is a system URL; `None` when it has no system scheme.
pub(super) fn parse_system_url(raw: &str) -> Result<Option<MediaSource>, String> {
    let Some((scheme, rest)) = raw.split_once("://") else {
        return Ok(None);
    };
    let url = match scheme.to_ascii_lowercase().as_str() {
        // `dfs:///<path>` is the location Files keeps for a folder.
        "cyfs" | "dfs" => parse_cyfs(rest),
        "file" | "obj" => buckyos_api::parse_object_url(raw),
        _ => return Ok(None),
    }
    .map_err(|err| format!("media `{raw}`: {err}"))?;
    let source = match url {
        ObjectUrl::CyfsPath { zone, path } => MediaSource::ZoneFile {
            zone,
            segments: url_segments(&path),
        },
        ObjectUrl::Obj {
            host: ObjHost::Service(service),
            path,
            ..
        } if service == "dfs" && !path.is_empty() => MediaSource::ZoneFile {
            zone: None,
            segments: url_segments(&path),
        },
        ObjectUrl::CyfsObject {
            obj_id, inner_path, ..
        } => {
            if !inner_path.is_empty() && inner_path != "/" {
                return Err(format!(
                    "media `{raw}`: a path inside object {obj_id} is not supported; pass the file's own object id"
                ));
            }
            MediaSource::Resource(ResourceRef::NamedObject { obj_id })
        }
        ObjectUrl::File {
            device_id: None,
            path,
        } => MediaSource::LocalFile(PathBuf::from(buckyos_api::decode_url_path(&path))),
        ObjectUrl::File {
            device_id: Some(device),
            path,
        } => {
            // Below the data root it is a Zone file, whichever device holds it
            // (key url.md D3).
            let local = PathBuf::from(buckyos_api::decode_url_path(&path));
            match ZoneView::from_runtime().zone_segments(&local) {
                Some(segments) => MediaSource::ZoneFile {
                    zone: None,
                    segments,
                },
                None => {
                    return Err(format!(
                        "media `{raw}` is a file of device {device}, not of this runtime: copy it into \
                         this session first (`agent-session copy --from <runtime id>:<path> --to session:<name>`) \
                         and pass the copy"
                    ))
                }
            }
        }
        ObjectUrl::Obj { .. } => {
            return Err(format!(
                "media `{raw}` names a service object, not a file: `read` it to find the file or object id it refers to"
            ))
        }
        other => return Err(format!("media `{raw}`: `{}` URLs are not media", other.scheme())),
    };
    Ok(Some(source))
}

/// `cyfs://<rest>`, strictly when it parses, else read the way Files and the
/// Desktop read it: `cyfs://home/a.pdf` lacks a slash — a Zone host always has
/// a dot, so `home` is where the path starts — and doubled slashes collapse.
fn parse_cyfs(rest: &str) -> Result<ObjectUrl, buckyos_api::ObjectUrlError> {
    let err = match buckyos_api::parse_object_url(&format!("cyfs://{rest}")) {
        Ok(url) => return Ok(url),
        Err(err) => err,
    };
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let is_object = |authority: &str| {
        matches!(
            buckyos_api::parse_object_url(&format!("cyfs://{authority}/")),
            Ok(ObjectUrl::CyfsObject { .. })
        )
    };
    if authority.contains('.') || is_object(authority) {
        return Err(err);
    }
    let authority = if authority == "_" { "" } else { authority };
    let joined = std::iter::once(authority)
        .chain(path.split('/'))
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        return Err(err);
    }
    buckyos_api::parse_object_url(&format!("cyfs:///{joined}")).map_err(|_| err)
}

/// The decoded segments of a canonical URL path.
fn url_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|segment| !segment.is_empty())
        .map(buckyos_api::decode_url_path)
        .collect()
}

/// `cyfs:///<segments>`.
pub(super) fn cyfs_url(segments: &[String]) -> String {
    let encoded: Vec<String> = segments
        .iter()
        .map(|segment| buckyos_api::encode_obj_segment(segment))
        .collect();
    format!("cyfs:///{}", encoded.join("/"))
}

/// A Zone file found here.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct ZoneFile {
    pub path: PathBuf,
    /// `cyfs:///…` of the file read.
    pub url: String,
    /// Set when the file read is not the path as written.
    pub note: Option<String>,
}

/// The Zone as this process sees it.
#[derive(Debug, Clone)]
pub(super) struct ZoneView {
    /// `$BUCKYOS_ROOT/data`: a DFS path is the file below it.
    pub data_root: PathBuf,
    pub zone_host: Option<String>,
    pub owner: Option<String>,
}

impl ZoneView {
    pub(super) fn from_runtime() -> Self {
        let runtime = buckyos_api::get_buckyos_api_runtime().ok();
        let data_root = buckyos_kit::get_buckyos_root_dir().join("data");
        let agent_root = std::env::var_os(crate::runtime_context::OPENDAN_AGENT_ROOT_ENV).map(PathBuf::from);
        // The Agent's owner (its app instance, or where its AgentRoot is), else
        // the user a client session logged in as. A service identity has
        // neither — its user id may be a device name — and no home is its own.
        let owner = runtime
            .and_then(|rt| rt.get_owner_user_id())
            .or_else(|| agent_root.as_deref().and_then(|root| home_owner(&data_root, root)))
            .or_else(|| {
                runtime
                    .filter(|rt| rt.runtime_type == buckyos_api::BuckyOSRuntimeType::AppClient)
                    .and_then(|rt| rt.authenticated_user_id.clone().or_else(|| rt.user_id.clone()))
            })
            .map(|owner| owner.trim().to_string())
            .filter(|owner| !owner.is_empty());
        Self {
            zone_host: runtime
                .and_then(|rt| rt.get_zone_config()?.zone_document().ok())
                .map(|document| buckyos_api::zone_document_hostname(&document).to_ascii_lowercase()),
            owner,
            data_root,
        }
    }

    /// The DFS segments of an absolute local path below the data root.
    fn zone_segments(&self, local: &Path) -> Option<Vec<String>> {
        let dfs = buckyos_api::data_path_to_dfs(&self.data_root, local)?;
        let segments: Vec<String> = dfs
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(str::to_string)
            .collect();
        (!segments.is_empty()).then_some(segments)
    }

    /// A bare absolute path that is no file here, read as the Zone path Files
    /// shows (`/home/<owner>/Documents/a.pdf`) when its first directory is one
    /// of the Zone's.
    pub(super) fn bare_path_segments(&self, raw: &Path) -> Option<Vec<String>> {
        if !raw.is_absolute() || raw.starts_with(&self.data_root) {
            return None;
        }
        let mut segments = Vec::new();
        for component in raw.components() {
            match component {
                Component::RootDir | Component::CurDir => {}
                Component::Normal(segment) => segments.push(segment.to_str()?.to_string()),
                _ => return None,
            }
        }
        let first = segments.first()?;
        self.data_root.join(first).is_dir().then_some(segments)
    }

    /// The local file a Zone path names here. `/home/<rest>` that is no file
    /// is also looked for in the owner's home, which prompts call `home`.
    pub(super) fn locate(&self, zone: Option<&str>, segments: &[String]) -> Result<ZoneFile, String> {
        let written = cyfs_url(segments);
        if let Some(zone) = zone {
            let here = self
                .zone_host
                .as_deref()
                .is_some_and(|host| host.eq_ignore_ascii_case(zone));
            if !here {
                return Err(format!(
                    "`cyfs://{zone}/…` is a file of another Zone, which this runtime does not read; \
                     ask for a share link (https://…) instead"
                ));
            }
        }
        let mut tried = Vec::new();
        for candidate in self.candidates(segments) {
            let Some(local) = self.local_path(&candidate) else {
                return Err(format!("`{written}` is not a plain Zone path"));
            };
            if local.is_file() {
                let url = cyfs_url(&candidate);
                let note = (candidate != segments).then(|| {
                    format!("`{written}` is not a file; read `{url}` in your owner's home instead.")
                });
                return Ok(ZoneFile { path: local, url, note });
            }
            if local.is_dir() && candidate == segments {
                return Err(format!(
                    "`{written}` is a folder (`{}` here); name a file in it",
                    local.display()
                ));
            }
            tried.push(local.display().to_string());
        }
        let in_home = segments.first().is_some_and(|first| first == "home");
        let hint = match (self.owner.as_deref(), self.owner_home()) {
            _ if !in_home => String::new(),
            (_, Some((url, local))) => format!(
                "; your owner's files are `{url}` (`{}` here): list that folder to find the file",
                local.display()
            ),
            (Some(owner), None) => format!(
                "; your owner's home `cyfs:///home/{}/` is not mounted in this runtime",
                buckyos_api::encode_obj_segment(owner)
            ),
            (None, None) => String::new(),
        };
        Err(format!(
            "Zone file `{written}` does not exist here (looked at {}){hint}",
            tried.join(", ")
        ))
    }

    fn candidates(&self, segments: &[String]) -> Vec<Vec<String>> {
        let mut candidates = vec![segments.to_vec()];
        if let (Some(owner), [first, second, ..]) = (self.owner.as_deref(), segments) {
            if first == "home" && second != owner {
                let mut mine = vec!["home".to_string(), owner.to_string()];
                mine.extend_from_slice(&segments[1..]);
                candidates.push(mine);
            }
        }
        candidates
    }

    fn local_path(&self, segments: &[String]) -> Option<PathBuf> {
        let mut path = self.data_root.clone();
        for segment in segments {
            if segment.is_empty()
                || segment == "."
                || segment == ".."
                || segment.contains(['/', '\\', '\0'])
            {
                return None;
            }
            path.push(segment);
        }
        Some(path)
    }

    fn owner_home(&self) -> Option<(String, PathBuf)> {
        let segments = vec!["home".to_string(), self.owner.clone()?];
        let local = self.local_path(&segments)?;
        local
            .is_dir()
            .then(|| (format!("{}/", cyfs_url(&segments)), local))
    }
}

/// `<owner>` of an AgentRoot `<data_root>/home/<owner>/agents/<agent_id>`.
fn home_owner(data_root: &Path, agent_root: &Path) -> Option<String> {
    let mut rest = agent_root.strip_prefix(data_root.join("home")).ok()?.components();
    let owner = match rest.next()? {
        Component::Normal(owner) => owner.to_str()?.to_string(),
        _ => return None,
    };
    matches!(rest.next(), Some(Component::Normal(dir)) if dir == "agents").then_some(owner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const OBJECT: &str = "cyfile:d05ec9f1d9ff3713fda374c2ad1e1a9ff1b168c941772df29769935fbbb49fdd";
    const BOOK: &str = "English for Everyone (Audio).pdf";

    fn zone_file(raw: &str) -> (Option<String>, Vec<String>) {
        match parse_system_url(raw).unwrap().unwrap() {
            MediaSource::ZoneFile { zone, segments } => (zone, segments),
            other => panic!("{raw}: {other:?}"),
        }
    }

    fn segs(path: &[&str]) -> Vec<String> {
        path.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn zone_paths_parse_in_the_spellings_people_use() {
        for raw in [
            "cyfs:///home/alice/English for Everyone (Audio).pdf",
            "cyfs:///home/alice/English%20for%20Everyone%20(Audio).pdf",
            "cyfs://_/home/alice/English for Everyone (Audio).pdf",
            "dfs:///home/alice/English for Everyone (Audio).pdf",
            "obj://dfs/home/alice/English%20for%20Everyone%20(Audio).pdf",
            // No third slash, doubled slashes.
            "cyfs://home/alice/English for Everyone (Audio).pdf",
            "cyfs:////home//alice/English for Everyone (Audio).pdf",
        ] {
            assert_eq!(zone_file(raw), (None, segs(&["home", "alice", BOOK])), "{raw}");
        }
        assert_eq!(
            zone_file("cyfs://test.buckyos.io/home/alice/a.pdf"),
            (Some("test.buckyos.io".to_string()), segs(&["home", "alice", "a.pdf"]))
        );
        assert_eq!(zone_file("cyfs://home/a.pdf").1, segs(&["home", "a.pdf"]));
    }

    #[test]
    fn object_and_file_urls_are_their_plain_forms() {
        let id = ndn_lib::ObjId::new(OBJECT).unwrap();
        match parse_system_url(&format!("cyfs://{}/", id.to_base32())).unwrap().unwrap() {
            MediaSource::Resource(ResourceRef::NamedObject { obj_id }) => assert_eq!(obj_id, id),
            other => panic!("{other:?}"),
        }
        assert!(parse_system_url(&format!("cyfs://{}/inner/a.png", id.to_base32())).is_err());
        match parse_system_url("file:///tmp/a%20b.png").unwrap().unwrap() {
            MediaSource::LocalFile(path) => assert_eq!(path, PathBuf::from("/tmp/a b.png")),
            other => panic!("{other:?}"),
        }
        let err = parse_system_url("obj://msg-center/inbox").unwrap_err();
        assert!(err.contains("`read` it"), "{err}");
        assert!(parse_system_url("https://example.com/a.png").unwrap().is_none());
        assert!(parse_system_url("/home/alice/a.png").unwrap().is_none());
    }

    fn zone(dir: &Path) -> ZoneView {
        ZoneView {
            data_root: dir.join("data"),
            zone_host: Some("test.buckyos.io".to_string()),
            owner: Some("alice".to_string()),
        }
    }

    #[test]
    fn a_zone_file_is_the_file_below_the_data_root() {
        let dir = tempfile::tempdir().unwrap();
        let zone = zone(dir.path());
        let home = zone.data_root.join("home/alice");
        fs::create_dir_all(home.join("docs")).unwrap();
        fs::write(home.join(BOOK), b"%PDF").unwrap();

        let found = zone.locate(None, &segs(&["home", "alice", BOOK])).unwrap();
        assert_eq!(found.path, home.join(BOOK));
        assert_eq!(found.url, "cyfs:///home/alice/English%20for%20Everyone%20(Audio).pdf");
        assert_eq!(found.note, None);
        assert_eq!(
            zone.locate(Some("TEST.buckyos.io"), &segs(&["home", "alice", BOOK])).unwrap().path,
            home.join(BOOK)
        );

        // `home` without a user is the owner's home, and the result says so.
        let found = zone.locate(None, &segs(&["home", BOOK])).unwrap();
        assert_eq!(found.path, home.join(BOOK));
        assert!(found.note.unwrap().contains("cyfs:///home/alice/English"));

        let err = zone.locate(None, &segs(&["home", "alice", "docs"])).unwrap_err();
        assert!(err.contains("is a folder"), "{err}");
        let err = zone.locate(None, &segs(&["home", "missing.pdf"])).unwrap_err();
        assert!(err.contains("cyfs:///home/alice/`"), "{err}");
        let elsewhere = ZoneView { owner: Some("bob".to_string()), ..zone.clone() };
        let err = elsewhere.locate(None, &segs(&["home", "missing.pdf"])).unwrap_err();
        assert!(err.contains("home/bob/missing.pdf") && err.contains("not mounted"), "{err}");
        let err = zone.locate(Some("other.zone.io"), &segs(&["home", "alice", BOOK])).unwrap_err();
        assert!(err.contains("another Zone"), "{err}");
        assert!(zone.locate(None, &segs(&["home", "..", "etc"])).is_err());
    }

    #[test]
    fn an_agent_root_names_its_owner() {
        let data = Path::new("/opt/buckyos/data");
        assert_eq!(
            home_owner(data, Path::new("/opt/buckyos/data/home/alice/agents/jarvis.test.buckyos.io")),
            Some("alice".to_string())
        );
        assert_eq!(home_owner(data, Path::new("/opt/buckyos/data/home/alice/Documents")), None);
        assert_eq!(home_owner(data, Path::new("/elsewhere/home/alice/agents/x")), None);
    }

    #[test]
    fn a_bare_path_is_a_zone_path_only_under_a_zone_directory() {
        let dir = tempfile::tempdir().unwrap();
        let zone = zone(dir.path());
        fs::create_dir_all(zone.data_root.join("home/alice")).unwrap();
        assert_eq!(
            zone.bare_path_segments(Path::new("/home/alice/a.pdf")),
            Some(segs(&["home", "alice", "a.pdf"]))
        );
        assert_eq!(zone.bare_path_segments(Path::new("/Users/alice/a.pdf")), None);
        assert_eq!(zone.bare_path_segments(Path::new("home/alice/a.pdf")), None);
        assert_eq!(zone.bare_path_segments(&zone.data_root.join("home/alice/a.pdf")), None);
        assert_eq!(
            zone.zone_segments(&zone.data_root.join("home/alice/a.pdf")),
            Some(segs(&["home", "alice", "a.pdf"]))
        );
    }
}
