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
    publish_pixa_with_state(&state, server, bytes_b64).await
}

/// Same native custody path without Tauri's injected `State` wrapper, for composed adapter
/// acceptance. Keep cancellation, unlock-generation and actor-incarnation fences here.
pub(super) async fn publish_pixa_with_state(
    state: &AppState,
    server: u64,
    bytes_b64: String,
) -> Result<creative_blobs::PixPublication, String> {
    let generation = unlocked_ui_session_generation(state).await?;
    let (lease, signal) = claim_internal_inline_download(state)?;
    let cancellation = RequestCancellation::new(signal, Some(lease.request_keepalive()));
    let bytes = decode_animation_input(&bytes_b64)?;
    let (actor, instance) = actor_instance_of(state, server).await?;
    let published = actor.publish_pixa(bytes, Some(cancellation)).await?;
    creative_blobs::finish_response(state, server, instance, generation, || published.into()).await
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
        downloads,
        gif,
        &cancellation,
        || {},
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

/// Stage, publish, then reveal. Only publication makes plaintext visible under the final name, so
/// only it runs under the UI-session guard and server-registry lock. Creating, writing and syncing
/// the `.part` on a slow Downloads volume holds neither, and runs off the async workers. Every
/// failure or cancellation before publication removes only the staging file.
/// `before_chunk` and `before_commit` are test seams; production passes no-ops.
#[allow(clippy::too_many_arguments)]
async fn save_gif<W, F, R>(
    state: &AppState,
    server: u64,
    instance: u64,
    generation: u64,
    downloads: PathBuf,
    gif: Vec<u8>,
    cancellation: &RequestCancellation,
    before_chunk: W,
    before_commit: F,
    reveal: R,
) -> Result<SavedFileResult, String>
where
    W: FnMut() + Send + 'static,
    F: Future<Output = ()>,
    R: FnOnce(&Path) -> Result<(), String>,
{
    if gif.len() > GIF_MAX_BYTES || !gif.starts_with(b"GIF89a") {
        return Err("invalid generated flipnote GIF".into());
    }
    let owned = cancellation.clone();
    // A worker that outlives this future still drops its unpublished result, which cleans up.
    let mut staged = tokio::task::spawn_blocking(move || {
        stage_gif(&downloads, &gif, || owned.is_cancelled(), before_chunk)
    })
    .await
    .map_err(|_| "GIF save worker failed".to_string())??;
    before_commit.await;
    let published = creative_blobs::finish_response(state, server, instance, generation, || {
        if cancellation.is_cancelled() {
            return Err("request cancelled".to_string());
        }
        staged.publish()
    })
    .await
    .and_then(|published| published);
    // Review L-1/M-5: settle the staging name on the blocking pool, outside the session guard and
    // the registry lock, published or not. The note says when a name had to stay.
    let path = staged.path.clone();
    let note = tokio::task::spawn_blocking(move || {
        if staged.published {
            staged.settle()
        } else {
            staged.discard()
        }
    })
    .await
    .unwrap_or_else(|_| {
        " Mewtual could not check the temporary GIF file, so it may have been left in place.".into()
    });
    if let Err(error) = published {
        return Err(format!("{error}{note}"));
    }
    // Revealing opens a system window, so it waits for no lock. It still belongs to the session
    // that started the export: a lock that won after the rename leaves the GIF in Downloads but
    // neither reveals it nor reports success behind the lock screen.
    drop(require_ui_session_generation(state, generation).await?);
    let mut saved = revealed(&path, reveal);
    if !note.is_empty() {
        private_downloads::append_warning(&mut saved, note.trim_start());
    }
    Ok(saved)
}

/// Bytes written and synced to a `.part` beside an absent final name. Publication atomically
/// refuses a destination created by another local action. Dropping it removes only staging.
///
/// PR51 H1/M1: the staging handle stays open for the whole life of the stage. Publication names
/// exactly its object (`publish_unjournaled`), and the staging name is only ever removed while it
/// still proves to be that object; a name swapped in meanwhile, or (Unix) a Downloads folder other
/// accounts can modify, is left alone. `save_gif` settles or discards the stage on the blocking
/// pool (review L-1); only a stage dropped on another path (a failure inside `stage_gif` or a
/// panicking worker) is cleaned up by `Drop`, silently.
struct StagedGif {
    file: std::fs::File,
    staging: PathBuf,
    path: PathBuf,
    published: bool,
    /// `settle` or `discard` already handled the staging name; `Drop` leaves it alone.
    settled: bool,
}

impl StagedGif {
    /// Publish exactly the staged object at the final name (PR51 M1). Only the link, which runs
    /// under the session guard and registry lock; the staging name is settled afterwards.
    fn publish(&mut self) -> Result<(), String> {
        let file = &self.file;
        publish_staged_download_with(&self.staging, &self.path, |from, to| {
            catcoms_storage::kept::publish_unjournaled(file, from, to)
        })?;
        self.published = true;
        Ok(())
    }

    /// After publication: flush the folder, remove the staging name (verified) and return the
    /// sentence to append when it had to stay (review M-5).
    fn settle(&mut self) -> String {
        self.settled = true;
        settle_published_staging(&self.file, &self.staging)
    }

    /// A failed or cancelled export: remove the unpublished staging name (verified) and return the
    /// sentence to append when it had to stay.
    fn discard(&mut self) -> String {
        self.settled = true;
        let removal = remove_staging(&self.file, &self.staging);
        let id = catcoms_storage::kept::session_id(&self.file).ok();
        left_in_place(&self.staging, &removal, id, Leftover::Partial)
    }
}

impl Drop for StagedGif {
    fn drop(&mut self) {
        if !self.published && !self.settled {
            // The handle is still open here (fields drop after this body), so the number the
            // removal verifies cannot belong to another file yet.
            let _ = remove_staging(&self.file, &self.staging);
        }
    }
}

/// One bounded write between cancellation checks. The whole GIF is at most 32 MiB.
const GIF_WRITE_CHUNK: usize = 1024 * 1024;

fn stage_gif(
    downloads: &Path,
    gif: &[u8],
    cancelled: impl Fn() -> bool,
    mut before_chunk: impl FnMut(),
) -> Result<StagedGif, String> {
    let (file, staging, path) = create_staged_download(downloads, GIF_NAME)?;
    let mut staged = StagedGif {
        file,
        staging,
        path,
        published: false,
        settled: false,
    };
    // A failure removes the staging name through its still-open handle (verified), and says so
    // when it had to stay (review M-5).
    if let Err(error) = write_staging(&mut staged.file, gif, &cancelled, &mut before_chunk) {
        return Err(format!("{error}{}", staged.discard()));
    }
    if cancelled() {
        return Err(format!("request cancelled{}", staged.discard()));
    }
    Ok(staged)
}

fn write_staging(
    file: &mut std::fs::File,
    gif: &[u8],
    cancelled: &impl Fn() -> bool,
    before_chunk: &mut impl FnMut(),
) -> Result<(), String> {
    for chunk in gif.chunks(GIF_WRITE_CHUNK) {
        before_chunk();
        if cancelled() {
            return Err("request cancelled".into());
        }
        file.write_all(chunk).map_err(|e| e.to_string())?;
    }
    file.sync_all().map_err(|e| e.to_string())
}

fn revealed<R: FnOnce(&Path) -> Result<(), String>>(path: &Path, reveal: R) -> SavedFileResult {
    let warning = reveal(path)
        .err()
        .map(|e| format!("The GIF was saved, but Downloads could not be opened: {e}"));
    SavedFileResult {
        path: path.to_string_lossy().into_owned(),
        displayed: warning.is_none(),
        warning,
        content_validation: Some("matched".into()),
        in_downloads: false,
    }
}

#[cfg(test)]
mod tests;
