use super::*;
use crate::catalog::PricingUnit;
use crate::execution::PinnedPricingBasis;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

fn requires_megapixels(record: &ExecutionRecord) -> bool {
    match record
        .binding
        .as_ref()
        .and_then(|binding| binding.pricing.as_ref())
        .map(|p| &p.basis)
    {
        Some(PinnedPricingBasis::Units { unit, addons, .. }) => {
            *unit == PricingUnit::Megapixel
                || addons.iter().any(|a| a.unit == PricingUnit::Megapixel)
        }
        Some(PinnedPricingBasis::Tokens { addons, .. }) => {
            addons.iter().any(|a| a.unit == PricingUnit::Megapixel)
        }
        None => false,
    }
}

fn positive_integer(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .filter(|v| *v > 0)
}

pub(super) fn pixel_measurement(value: &Value, counted: bool) -> Result<Value, BillingLookupError> {
    let stream = value
        .get("streams")
        .and_then(Value::as_array)
        .and_then(|s| s.first())
        .ok_or("probe_missing_video")?;
    let width = positive_integer(stream.get("width")).ok_or("probe_missing_dimensions")?;
    let height = positive_integer(stream.get("height")).ok_or("probe_missing_dimensions")?;
    let frames = positive_integer(stream.get(if counted {
        "nb_read_frames"
    } else {
        "nb_frames"
    }))
    .ok_or("probe_missing_frames")?;
    let pixels = width
        .checked_mul(height)
        .and_then(|p| p.checked_mul(frames))
        .filter(|p| *p <= (1u64 << 53))
        .ok_or("probe_invalid_pixels")?;
    Ok(
        json!({"width":width,"height":height,"frames":frames,"pixels":pixels,"megapixels":pixels as f64 / 1_000_000.0,"method":if counted {"decoded_frames"} else {"video_header"}}),
    )
}

struct ProbeFile(PathBuf);
impl Drop for ProbeFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

async fn probe_video(bytes: &[u8], task: &str, counted: bool) -> Result<Value, BillingLookupError> {
    let path = std::env::temp_dir().join(format!(
        "aicc-billing-{:x}.mp4",
        Sha256::digest(task.as_bytes())
    ));
    let _ = tokio::fs::remove_file(&path).await;
    let mut options = tokio::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(&path)
        .await
        .map_err(|_| BillingLookupError::from("probe_io"))?;
    let guard = ProbeFile(path);
    file.write_all(bytes)
        .await
        .map_err(|_| BillingLookupError::from("probe_io"))?;
    file.flush()
        .await
        .map_err(|_| BillingLookupError::from("probe_io"))?;
    drop(file);
    let installed = buckyos_kit::get_buckyos_root_dir()
        .join("tools/bin")
        .join(if cfg!(windows) {
            "ffprobe.exe"
        } else {
            "ffprobe"
        });
    let mut command = tokio::process::Command::new(if installed.is_file() {
        installed
    } else {
        PathBuf::from("ffprobe")
    });
    command.kill_on_drop(true).stdin(Stdio::null()).args([
        "-v",
        "error",
        "-protocol_whitelist",
        "file,pipe",
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,nb_frames,nb_read_frames",
        "-of",
        "json",
    ]);
    if counted {
        command.arg("-count_frames");
    }
    let output = tokio::time::timeout(
        Duration::from_secs(if counted { 20 } else { 10 }),
        command.arg("-i").arg(&guard.0).output(),
    )
    .await
    .map_err(|_| BillingLookupError::from("probe_timeout"))?
    .map_err(|_| BillingLookupError::from("probe_unavailable"))?;
    if counted && !output.status.success() {
        return Err("probe_invalid_video".into());
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| BillingLookupError::from("probe_invalid_video"))?;
    pixel_measurement(&value, counted)
}

impl RuntimeProviderExecutionPort {
    pub(super) async fn local_billing_usage(
        &self,
        record: &ExecutionRecord,
    ) -> Result<(buckyos_api::AiUsage, Option<Value>), BillingLookupError> {
        let output = record.output.as_ref().ok_or("missing_output")?;
        let mut usage = output.usage.clone();
        if !requires_megapixels(record) || usage.megapixels.is_some() {
            return Ok((usage, None));
        }
        let artifact = output
            .artifacts
            .iter()
            .find(|a| {
                a.mime
                    .as_deref()
                    .is_some_and(|mime| mime.starts_with("video/"))
            })
            .ok_or("probe_missing_artifact")?;
        let limit = crate::resource::ResourceLimits::default().max_total_bytes;
        let (prefix, complete) = match &artifact.resource {
            buckyos_api::ResourceRef::Url { url, .. } => {
                let mut reader = self
                    .open_artifact_url_reader(&record.scope.tenant_id, &record.user_id, url, None)
                    .await
                    .map_err(|_| BillingLookupError::from("probe_download_failed"))?;
                let mut bytes = Vec::new();
                let mut complete = false;
                while (bytes.len() as u64) < ndn_lib::MEDIA_META_READ_LIMIT {
                    let Some(chunk) = reader.body.next().await else {
                        complete = true;
                        break;
                    };
                    let chunk =
                        chunk.map_err(|_| BillingLookupError::from("probe_download_failed"))?;
                    let take = chunk
                        .len()
                        .min((ndn_lib::MEDIA_META_READ_LIMIT - bytes.len() as u64) as usize);
                    bytes.extend_from_slice(&chunk[..take]);
                }
                (bytes, complete)
            }
            buckyos_api::ResourceRef::NamedObject { obj_id } => {
                let bytes = self
                    .resource_store
                    .read(obj_id, limit)
                    .await
                    .map_err(|_| BillingLookupError::from("probe_download_failed"))?;
                (bytes, true)
            }
            _ => return Err("probe_invalid_resource".into()),
        };
        let mut measured = match probe_video(&prefix, &record.task_id, false).await {
            Ok(measured) => measured,
            Err(_) => {
                let bytes = if complete {
                    prefix
                } else {
                    let buckyos_api::ResourceRef::Url { url, .. } = &artifact.resource else {
                        return Err("probe_invalid_resource".into());
                    };
                    let url = reqwest::Url::parse(url)
                        .map_err(|_| BillingLookupError::from("probe_invalid_resource"))?;
                    self.fetch_artifact_url_resource(
                        &record.scope.tenant_id,
                        &record.user_id,
                        &url,
                        limit,
                    )
                    .await
                    .map_err(|_| BillingLookupError::from("probe_download_failed"))?
                    .bytes
                };
                probe_video(&bytes, &record.task_id, true).await?
            }
        };
        usage.megapixels = measured["megapixels"].as_f64();
        measured["artifact_name"] = json!(artifact.name);
        usage.cost_unknown_reason = None;
        Ok((usage, Some(measured)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_pixels_use_actual_frame_counts_without_assuming_constant_fps() {
        let header = json!({"streams":[{"width":960,"height":540,"nb_frames":"150","duration":"5.005","avg_frame_rate":"30000/1001"}]});
        let value = pixel_measurement(&header, false).unwrap();
        assert_eq!(value["frames"], 150);
        assert_eq!(value["megapixels"], 77.76);
        assert_eq!(value["method"], "video_header");
        let full = json!({"streams":[{"width":960,"height":540,"nb_frames":"9999","nb_read_frames":"150"}]});
        assert_eq!(pixel_measurement(&full, true).unwrap()["megapixels"], 77.76);
        for bad in [
            json!({"streams":[{"width":960,"height":540,"nb_frames":"N/A"}]}),
            json!({"streams":[{"width":960,"height":540,"nb_frames":"0"}]}),
            json!({"streams":[{"width":4294967296u64,"height":4294967296u64,"nb_frames":"150"}]}),
        ] {
            assert!(pixel_measurement(&bad, false).is_err());
        }
    }
}
