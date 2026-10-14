//! Narrow native animation exports. PIXA publication is immutable content only; GIF is an
//! explicit local visual copy, never a replicated reference and never an audio recording.

use super::*;
use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame, RgbaImage};

const GIF_MAX_BYTES: usize = 32 * 1024 * 1024;
const GIF_NAME: &str = "Mewtual-flipnote.gif";

fn decode_animation_input(encoded: &str) -> Result<Vec<u8>, String> {
    let max = catcoms_app::creative::MAX_BOUNDED_BLOB_BYTES;
    if encoded.len() > max.div_ceil(3) * 4 {
        return Err("PIXA blob exceeds 9 MiB".into());
    }
    let bytes = B64
        .decode(encoded)
        .map_err(|_| "invalid PIXA base64".to_string())?;
    if bytes.len() > max {
        return Err("PIXA blob exceeds 9 MiB".into());
    }
    Ok(bytes)
}

/// Publish only a completely validated animation, promoted and flushed by the existing actor
/// blob store. Saving its SetExport metadata is a later separate transaction.
#[tauri::command]
pub(crate) async fn publish_pixa(
    state: State<'_, AppState>,
    server: u64,
    bytes_b64: String,
) -> Result<creative_blobs::PixPublication, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let (lease, signal) = claim_internal_inline_download(&state)?;
    let cancellation = RequestCancellation::new(signal, Some(lease.request_keepalive()));
    let bytes = decode_animation_input(&bytes_b64)?;
    let (actor, instance) = actor_instance_of(&state, server).await?;
    let published = actor.publish_pixa(bytes, Some(cancellation)).await?;
    creative_blobs::finish_response(&state, server, instance, generation, || published.into()).await
}

/// Locally encode the validated visual frames. The fixed name accepts no renderer path, and the
/// exact unlocked session is checked again after detached encoding before publishing/revealing.
#[tauri::command]
pub(crate) async fn export_studio_gif(
    app: AppHandle,
    state: State<'_, AppState>,
    server: u64,
    bytes_b64: String,
) -> Result<SavedFileResult, String> {
    let generation = unlocked_ui_session_generation(&state).await?;
    let (lease, signal) = claim_internal_inline_download(&state)?;
    let cancellation = RequestCancellation::new(signal, Some(lease.request_keepalive()));
    let (_, instance) = actor_instance_of(&state, server).await?;
    let bytes = decode_animation_input(&bytes_b64)?;
    let owned = cancellation.clone();
    let gif = tokio::task::spawn_blocking(move || {
        encode_gif(&bytes, GIF_MAX_BYTES, || owned.is_cancelled())
    })
    .await
    .map_err(|_| "GIF encoding worker failed".to_string())??;
    let downloads = app.path().download_dir().map_err(|e| e.to_string())?;
    save_gif(
        &state,
        server,
        instance,
        generation,
        &downloads,
        &gif,
        &cancellation,
        std::future::ready(()),
        reveal_path,
    )
    .await
}

struct BoundedGif {
    bytes: Vec<u8>,
    limit: usize,
    failed: bool,
}
impl std::io::Write for BoundedGif {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            self.failed = true;
            return Err(std::io::Error::other("GIF output exceeds 32 MiB"));
        }
        // Bound requested capacity as well as logical length. Plain Vec growth can double past
        // the output ceiling just below the final admitted length.
        let needed = self.bytes.len() + bytes.len();
        if self.bytes.capacity() < needed {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(needed)
                .min(self.limit);
            if self
                .bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .is_err()
            {
                self.failed = true;
                return Err(std::io::Error::other("GIF output allocation refused"));
            }
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn encode_gif(bytes: &[u8], limit: usize, cancelled: impl Fn() -> bool) -> Result<Vec<u8>, String> {
    if cancelled() {
        return Err("request cancelled".into());
    }
    let pixa = catcoms_app::creative::validate_pixa(bytes).map_err(|e| e.to_string())?;
    let mut output = BoundedGif {
        bytes: Vec::new(),
        limit: limit.min(GIF_MAX_BYTES),
        failed: false,
    };
    {
        let mut encoder = GifEncoder::new(&mut output);
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        for (index, pix) in pixa.frames().iter().enumerate() {
            if cancelled() {
                return Err("request cancelled".into());
            }
            // Validation already proved exact dimensions/runs. Expand only this frame. Roles
            // carry receiver-theme semantics in the editor; local portable GIF uses declared RGB.
            let palette = usize::from(pix[6]) + 1;
            let mut raster = Vec::with_capacity(192 * 144 * 4);
            for run in pix[7 + 4 * palette..].chunks_exact(2) {
                let color = 7 + 4 * usize::from(run[1]);
                for _ in 0..=run[0] {
                    raster.extend([pix[color + 1], pix[color + 2], pix[color + 3], 255]);
                }
            }
            let image = RgbaImage::from_raw(192, 144, raster)
                .ok_or_else(|| "invalid GIF raster".to_string())?;
            let fps = usize::from(pixa.fps());
            // GIF has centisecond resolution. Carry fractional timing across frames, never
            // round each frame independently (which would accumulate drift at12/24FPS).
            let centiseconds = ((index + 1) * 100 / fps) - (index * 100 / fps);
            encoder
                .encode_frame(Frame::from_parts(
                    image,
                    0,
                    0,
                    Delay::from_numer_denom_ms((centiseconds * 10) as u32, 1),
                ))
                .map_err(|e| e.to_string())?;
        }
    }
    // The image encoder finalizes its trailer on Drop, which cannot return an I/O error. The
    // writer remembers even that final failed write so a truncated-at-cap GIF never succeeds.
    if output.failed {
        return Err("GIF output exceeds 32 MiB".into());
    }
    if cancelled() {
        return Err("request cancelled".into());
    }
    Ok(output.bytes)
}

#[allow(clippy::too_many_arguments)]
async fn save_gif<F, R>(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    downloads: &Path,
    gif: &[u8],
    cancellation: &RequestCancellation,
    before_commit: F,
    reveal: R,
) -> Result<SavedFileResult, String>
where
    F: Future<Output = ()>,
    R: FnOnce(&Path) -> Result<(), String>,
{
    if gif.len() > GIF_MAX_BYTES || !gif.starts_with(b"GIF89a") {
        return Err("invalid generated flipnote GIF".into());
    }
    before_commit.await;
    creative_blobs::finish_response(state, server, instance, generation, || {
        if cancellation.is_cancelled() {
            return Err("request cancelled".into());
        }
        write_gif(downloads, gif, reveal)
    })
    .await?
}

fn write_gif<R: FnOnce(&Path) -> Result<(), String>>(
    downloads: &Path,
    gif: &[u8],
    reveal: R,
) -> Result<SavedFileResult, String> {
    let (mut file, staging, path) = create_staged_download(downloads, GIF_NAME)?;
    let result = file
        .write_all(gif)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string());
    drop(file);
    if let Err(error) = result.and_then(|()| publish_staged_download(&staging, &path)) {
        let _ = std::fs::remove_file(&staging);
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    let warning = reveal(&path)
        .err()
        .map(|e| format!("The GIF was saved, but Downloads could not be opened: {e}"));
    Ok(SavedFileResult {
        path: path.to_string_lossy().into_owned(),
        displayed: warning.is_none(),
        warning,
        content_validation: Some("matched".into()),
    })
}

#[cfg(test)]
mod tests;
