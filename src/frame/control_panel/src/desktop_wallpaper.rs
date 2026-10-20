/*
Desktop wallpaper pictures of the signed-in user, part of the desktop UI state (`/api/desktop`).

The account's appearance state (`users/{user}/desktop/{session}/appearance`) only names the picture;
the bytes are a file in the user's home, so the picture follows the account to every device:

  data/home/{user}/.local/share/control-panel/wallpapers/{sha256[..32]}.{ext}

- `POST /api/desktop/wallpaper`  body = the image bytes (JPEG/PNG/WebP/GIF/AVIF, ≤ 16 MiB)
  → `{ok, data: {asset, bytes}}`. The newest few pictures are kept, older ones deleted.
- `GET /api/desktop/wallpaper/{asset}` → the bytes. Names are content hashes, so the answer is
  cached as immutable. A CSS/`<img>` load sends no headers, so a read also takes the Gateway SSO
  session cookie (`buckyos_session_token`, the one `/sso_refresh` keeps fresh at the Zone root).
  An upload does not: it changes the account, and the cookie also rides along on requests other
  apps of the same site make.
*/

use super::download_mgr::home_segment;
use super::ControlPanelServer;
use buckyos_http_server::*;
use buckyos_kit::get_buckyos_root_dir;
use bytes::Bytes;
use http::{Method, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Limited};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub(super) const WALLPAPER_ROUTE: &str = "/api/desktop/wallpaper";
const MAX_WALLPAPER_BYTES: usize = 16 * 1024 * 1024;
/// Pictures kept besides the newest: a device that has not refreshed its state yet still finds the one it shows.
const KEPT_WALLPAPERS: usize = 3;
const WALLPAPER_EXTENSIONS: [&str; 5] = ["jpg", "png", "gif", "webp", "avif"];

/// The picture's type from its leading bytes; anything else is refused.
pub(super) fn wallpaper_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && matches!(&bytes[8..12], b"avif" | b"avis") {
        Some("avif")
    } else {
        None
    }
}

fn content_type(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        _ => "image/jpeg",
    }
}

pub(super) fn wallpaper_asset_name(bytes: &[u8], extension: &str) -> String {
    let digest = hex::encode(Sha256::digest(bytes));
    format!("{}.{}", &digest[..32], extension)
}

/// `{32 lowercase hex}.{ext}` and nothing else, so a name never leaves the wallpaper folder.
pub(super) fn is_wallpaper_asset_name(name: &str) -> bool {
    let Some((stem, extension)) = name.split_once('.') else {
        return false;
    };
    stem.len() == 32
        && stem.bytes().all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        && WALLPAPER_EXTENSIONS.contains(&extension)
}

fn wallpaper_dir(username: &str) -> Option<PathBuf> {
    let home = home_segment(username)?;
    Some(
        get_buckyos_root_dir()
            .join("data")
            .join("home")
            .join(home)
            .join(".local")
            .join("share")
            .join("control-panel")
            .join("wallpapers"),
    )
}

/// Pictures to delete: all but the newest `keep`, never `current`.
pub(super) fn wallpapers_to_prune(
    mut pictures: Vec<(String, std::time::SystemTime)>,
    current: &str,
    keep: usize,
) -> Vec<String> {
    pictures.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pictures
        .into_iter()
        .filter(|(name, _)| name != current)
        .skip(keep.saturating_sub(1))
        .map(|(name, _)| name)
        .collect()
}

async fn prune_wallpapers(dir: &Path, current: &str) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    let mut pictures = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        if !is_wallpaper_asset_name(&name) {
            continue;
        }
        if let Ok(modified) = entry.metadata().await.and_then(|meta| meta.modified()) {
            pictures.push((name, modified));
        }
    }
    for name in wallpapers_to_prune(pictures, current, KEPT_WALLPAPERS) {
        if let Err(error) = tokio::fs::remove_file(dir.join(&name)).await {
            log::warn!("desktop: remove old wallpaper {}: {}", name, error);
        }
    }
}

impl ControlPanelServer {
    /// An explicit token (header, query) first; a read then prefers the SSO session cookie, which stays
    /// fresh, over an older `control-panel_token` cookie a password sign-in may have left behind.
    pub(super) fn wallpaper_request_token(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Option<String> {
        let headers = req.headers();
        let explicit = headers.contains_key("X-Auth")
            || headers.contains_key(http::header::AUTHORIZATION)
            || req.uri().query().is_some_and(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .any(|(key, _)| key == "auth" || key == "session_token")
            });
        if !explicit && req.method() == Method::GET {
            if let Some(token) = Self::extract_http_sso_session_cookie(req) {
                return Some(token);
            }
        }
        Self::extract_http_session_token(req)
    }

    pub(super) async fn handle_desktop_wallpaper(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let token = Self::wallpaper_request_token(&req);
        let principal = match self
            .authenticate_session_token_for_method("desktop.api", token)
            .await
        {
            Ok(Some(principal)) => principal,
            Ok(None) => return Self::wallpaper_error(StatusCode::UNAUTHORIZED, "authentication required"),
            Err(error) => return Self::wallpaper_error(StatusCode::UNAUTHORIZED, &error.to_string()),
        };
        let Some(dir) = wallpaper_dir(&principal.username) else {
            return Self::wallpaper_error(StatusCode::BAD_REQUEST, "this account has no home folder");
        };

        let path = req.uri().path().to_string();
        let method = req.method().clone();
        let permission_method = if method == Method::GET {
            "desktop.wallpaper.get"
        } else {
            "desktop.wallpaper.write"
        };
        if let Err(error) = principal.require_agent_permission(permission_method) {
            return Self::wallpaper_error(StatusCode::FORBIDDEN, &error.to_string());
        }
        if path == WALLPAPER_ROUTE && method == Method::POST {
            return self.store_wallpaper(req, &dir).await;
        }
        if let Some(asset) = path.strip_prefix(&format!("{}/", WALLPAPER_ROUTE)) {
            if method == Method::GET {
                return Self::serve_wallpaper(&dir, asset).await;
            }
        }
        Self::wallpaper_error(StatusCode::METHOD_NOT_ALLOWED, "use POST to upload, GET to read")
    }

    async fn store_wallpaper(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        dir: &Path,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let bytes = match Limited::new(req.into_body(), MAX_WALLPAPER_BYTES).collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(_) => {
                return Self::wallpaper_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "the picture is larger than 16 MiB",
                )
            }
        };
        let Some(extension) = wallpaper_extension(&bytes) else {
            return Self::wallpaper_error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "the picture must be JPEG, PNG, WebP, GIF or AVIF",
            );
        };
        let asset = wallpaper_asset_name(&bytes, extension);

        let stored = async {
            tokio::fs::create_dir_all(dir).await?;
            // Written even when the same picture is there, so its time makes it the newest again.
            let partial = dir.join(format!(".{}.part", asset));
            tokio::fs::write(&partial, &bytes).await?;
            tokio::fs::rename(&partial, dir.join(&asset)).await
        }
        .await;
        if let Err(error) = stored {
            log::error!("desktop: store wallpaper in {}: {}", dir.display(), error);
            return Self::wallpaper_error(StatusCode::INTERNAL_SERVER_ERROR, "the picture could not be saved");
        }
        prune_wallpapers(dir, &asset).await;

        Self::build_http_json_response(
            StatusCode::OK,
            json!({"ok": true, "data": {"asset": asset, "bytes": bytes.len()}}),
        )
    }

    async fn serve_wallpaper(
        dir: &Path,
        asset: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if !is_wallpaper_asset_name(asset) {
            return Self::wallpaper_error(StatusCode::NOT_FOUND, "no such picture");
        }
        let bytes = match tokio::fs::read(dir.join(asset)).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self::wallpaper_error(StatusCode::NOT_FOUND, "no such picture")
            }
            Err(error) => {
                log::error!("desktop: read wallpaper {}: {}", asset, error);
                return Self::wallpaper_error(StatusCode::INTERNAL_SERVER_ERROR, "the picture could not be read");
            }
        };
        let extension = asset.rsplit('.').next().unwrap_or("jpg");
        http::Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", content_type(extension))
            .header("Content-Length", bytes.len())
            // The name is the content hash; `private` keeps shared caches out of a signed-in answer.
            .header("Cache-Control", "private, max-age=31536000, immutable")
            .body(Self::boxed_http_body(bytes))
            .map_err(|error| server_err!(ServerErrorCode::BadRequest, "build response: {}", error))
    }

    fn wallpaper_error(
        status: StatusCode,
        message: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        Self::build_http_json_response(status, json!({"ok": false, "error": message}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn picture_types_come_from_the_bytes() {
        assert_eq!(wallpaper_extension(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some("jpg"));
        assert_eq!(wallpaper_extension(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(wallpaper_extension(b"GIF89a......"), Some("gif"));
        assert_eq!(wallpaper_extension(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(wallpaper_extension(b"\0\0\0\x1cftypavif\0\0"), Some("avif"));
        assert_eq!(wallpaper_extension(b"<svg xmlns=..."), None);
        assert_eq!(wallpaper_extension(b""), None);
    }

    #[test]
    fn asset_names_are_content_hashes_and_cannot_escape_the_folder() {
        let name = wallpaper_asset_name(b"picture", "png");
        assert!(is_wallpaper_asset_name(&name));
        assert_eq!(name, wallpaper_asset_name(b"picture", "png"));
        assert_ne!(name, wallpaper_asset_name(b"other", "png"));
        assert!(!is_wallpaper_asset_name("../settings"));
        assert!(!is_wallpaper_asset_name(&format!("{}.svg", "a".repeat(32))));
        assert!(!is_wallpaper_asset_name(&format!("{}.png", "A".repeat(32))));
        assert!(!is_wallpaper_asset_name(&format!("{}.png", "a".repeat(31))));
        assert!(!is_wallpaper_asset_name(&format!("{}.png.part", "a".repeat(32))));
    }

    fn wallpaper_request(method: Method, uri: &str, headers: &[(&str, &str)]) -> http::Request<BoxBody<Bytes, ServerError>> {
        let mut builder = http::Request::builder().method(method).uri(uri);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder
            .body(ControlPanelServer::boxed_http_body(Vec::new()))
            .unwrap()
    }

    #[test]
    fn a_picture_read_takes_the_sso_session_cookie_an_upload_does_not() {
        let cookies = "control-panel_token=stale-password-login; buckyos_session_token=fresh-sso-session";
        let read = wallpaper_request(Method::GET, "/api/desktop/wallpaper/x.jpg", &[("Cookie", cookies)]);
        assert_eq!(ControlPanelServer::wallpaper_request_token(&read).as_deref(), Some("fresh-sso-session"));

        let only_sso = wallpaper_request(Method::GET, "/api/desktop/wallpaper/x.jpg", &[("Cookie", "buckyos_session_token=sso")]);
        assert_eq!(ControlPanelServer::wallpaper_request_token(&only_sso).as_deref(), Some("sso"));

        let with_header = wallpaper_request(Method::GET, "/api/desktop/wallpaper/x.jpg", &[("Cookie", cookies), ("X-Auth", "header-token")]);
        assert_eq!(ControlPanelServer::wallpaper_request_token(&with_header).as_deref(), Some("header-token"));

        let upload = wallpaper_request(Method::POST, "/api/desktop/wallpaper", &[("Cookie", "buckyos_session_token=sso")]);
        assert_eq!(ControlPanelServer::wallpaper_request_token(&upload), None);
    }

    #[test]
    fn only_the_newest_pictures_are_kept() {
        let at = |secs| SystemTime::UNIX_EPOCH + Duration::from_secs(secs);
        let pictures = vec![
            ("a".to_string(), at(1)),
            ("b".to_string(), at(4)),
            ("c".to_string(), at(3)),
            ("d".to_string(), at(2)),
            ("new".to_string(), at(5)),
        ];
        assert_eq!(wallpapers_to_prune(pictures.clone(), "new", 3), vec!["d", "a"]);
        // The current picture is kept even when its file is not the newest.
        assert_eq!(wallpapers_to_prune(pictures, "a", 3), vec!["c", "d"]);
    }
}
