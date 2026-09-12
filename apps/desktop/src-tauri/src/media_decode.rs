//! Decode shared images in Rust and hand the webview a PNG that Mewtual wrote, not one a peer did.
//!
//! # The bug class this exists for
//!
//! Inline images auto-load: a member posts a file, everyone else's client fetches it and paints it
//! with no gesture, inside the single privileged `main` webview. The validators in `lib.rs`
//! (`safe_media_mime`, `detected_media_container`, `validated_inline_media_mime`) make sure the
//! bytes are a media container we are willing to name, that the name and the magic agree, and that
//! the response carries `nosniff` and `no-store`. None of that helps against the file that is a
//! *perfectly valid* PNG or AVIF and still corrupts memory inside the platform image decoder,
//! because there is nothing malformed to reject. The only move left is to stop giving the platform
//! decoder attacker-controlled bytes at all.
//!
//! So: the peer's bytes are decoded here, by pure-Rust decoders, and what crosses the scheme
//! boundary is a PNG re-encoded from the finished pixels. WebView2 still runs a PNG decoder, but on
//! a stream produced by [`encode_png`] below, whose shape is fixed by us: 8-bit, non-interlaced,
//! truecolour or truecolour-alpha, no palette, no ancillary chunks, no ICC, no text. An attacker
//! controls the pixel values and (within the bounds below) the dimensions. Nothing else.
//!
//! # What this does NOT do
//!
//! - It is not a general safety claim about media. Audio and video still stream raw to the platform
//!   decoders; those formats have no pure-Rust decoder we would ship, and this module does not
//!   touch them.
//! - It does not remove denial of service. A memory-safe decoder still allocates, still spins, and
//!   now does so *in our process* rather than in the webview's. That is what [`DecodeBounds`] is
//!   for, and the bounds are honest about which of them are enforced before the cost is paid and
//!   which are only noticed afterwards.
//! - It does not make the pure-Rust decoders correct. They can still panic on hostile input;
//!   [`transcode_inline_image_with`] catches unwinds so a panic becomes a refusal rather than a
//!   dead scheme handler. That containment depends on the build unwinding: if a `panic = "abort"`
//!   profile is ever added to this crate, the catch becomes a no-op and a decoder panic takes the
//!   whole app down.
//! - It is an additional stage, not a replacement. The container check in `lib.rs` still runs
//!   first, and this module deliberately never sniffs: it decodes with the decoder the *declared*
//!   type names, so a body disagreeing with its declaration fails here too rather than being
//!   quietly decoded as whatever it really is.
//!
//! # Formats
//!
//! Every format the inline allowlist admits has a pure-Rust decoder in the `image` crate except
//! AVIF, whose only decoders are `dav1d` (a binding to the C libdav1d) and `rav1d` (a transpile of
//! that same C, unsafe throughout, and not wired into `image`). Neither buys what this module is
//! for, so AVIF is not transcodable here and [`SourceFormat::from_mime`] refuses it.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::io::{self, Cursor, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::time::{Duration, Instant};

use image::codecs::bmp::BmpDecoder;
use image::codecs::gif::GifDecoder;
use image::codecs::ico::IcoDecoder;
use image::codecs::jpeg::JpegDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::tiff::TiffDecoder;
use image::codecs::webp::WebPDecoder;
use image::error::ImageError;
use image::{AnimationDecoder, DynamicImage, Frames, ImageDecoder, Limits};

/// The one MIME type this module ever produces. Fixed rather than derived: the whole point is that
/// the webview sees exactly one container, written by one encoder, in one shape.
pub const TRANSCODED_MIME: &str = "image/png";

/// An inline image format we are willing to decode ourselves.
///
/// Membership is the answer to one question and no other: does a pure-Rust decoder for this exist
/// in the crate graph? `image/avif` is absent for that reason, not because the format is unpopular.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
    Tiff,
    Ico,
}

impl SourceFormat {
    /// Map a declared MIME type to the decoder we will run for it.
    ///
    /// The declaration is normalised the same way `safe_media_mime` normalises it (trimmed,
    /// lowercased, parameters dropped) so this agrees with the validator that ran first even if a
    /// caller hands us the raw manifest value by mistake.
    ///
    /// Returning `None` is not "unknown": it is a decision that these bytes must not be handed to
    /// the platform decoder either. Callers fail closed.
    #[must_use]
    pub fn from_mime(declared: &str) -> Option<Self> {
        let lowered = declared.trim().to_ascii_lowercase();
        let base = lowered.split(';').next().unwrap_or("").trim().to_string();
        match base.as_str() {
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/gif" => Some(Self::Gif),
            "image/webp" => Some(Self::Webp),
            "image/bmp" => Some(Self::Bmp),
            "image/tiff" => Some(Self::Tiff),
            "image/x-icon" => Some(Self::Ico),
            _ => None,
        }
    }

    /// The MIME type this format is admitted under, for logs and tests.
    #[must_use]
    pub fn as_mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Bmp => "image/bmp",
            Self::Tiff => "image/tiff",
            Self::Ico => "image/x-icon",
        }
    }

    /// Whether the container can carry more than one frame. GIF always can; WebP only in its
    /// extended form, which is why the WebP path asks the decoder rather than assuming.
    fn may_animate(self) -> bool {
        matches!(self, Self::Gif | Self::Webp)
    }
}

/// Why a transcode refused. Every variant is a "show the click-to-load chip" outcome for the
/// caller; they are distinguished so the log says which bound or which stage refused, because
/// "image did not appear" with no reason is the failure mode that wastes an afternoon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecodeRefusal {
    /// The declared type is not one we can decode in Rust (AVIF, audio, video, anything else).
    UnsupportedType,
    /// The encoded file is larger than [`DecodeBounds::max_input_bytes`].
    InputTooLarge,
    /// A side exceeds [`DecodeBounds::max_dimension`].
    DimensionsOverBound,
    /// Width times height exceeds [`DecodeBounds::max_pixels`], or a decoder hit its own limit.
    PixelsOverBound,
    /// A container we recognise, in a variant this decoder cannot read.
    UnsupportedEncoding,
    /// Truncated, corrupt, or not the format its declaration claimed.
    Malformed,
    /// The decode ran past [`DecodeBounds::max_duration`].
    DeadlineExceeded,
    /// The re-encoded PNG would exceed [`DecodeBounds::max_output_bytes`].
    OutputOverBound,
    /// The encoder failed for a reason that is our bug, not the input's.
    EncodeFailed,
    /// A decoder panicked and the unwind was contained here. Always worth a log line: it is a
    /// reachable bug in a dependency, reached by a peer's file.
    DecoderPanicked,
}

impl DecodeRefusal {
    /// A stable short token, for logs and for a response header if one is ever wanted.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedType => "unsupported-type",
            Self::InputTooLarge => "input-too-large",
            Self::DimensionsOverBound => "dimensions-over-bound",
            Self::PixelsOverBound => "pixels-over-bound",
            Self::UnsupportedEncoding => "unsupported-encoding",
            Self::Malformed => "malformed",
            Self::DeadlineExceeded => "deadline-exceeded",
            Self::OutputOverBound => "output-over-bound",
            Self::EncodeFailed => "encode-failed",
            Self::DecoderPanicked => "decoder-panicked",
        }
    }
}

impl fmt::Display for DecodeRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Resource ceilings for one transcode.
///
/// Memory safety is not the same as safety. Everything below exists because doing the decode
/// in-process turns a webview problem into an app problem: a 60000x60000 image, a zlib bomb or a
/// pathological progressive JPEG now spends *our* memory and *our* CPU, and the actor behind the
/// media scheme is the same one answering `get_messages`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeBounds {
    /// Largest encoded file we will even look at. Above this the caller falls back to the chip
    /// rather than the old raw-bytes path, because serving raw bytes is the hole this closes.
    pub max_input_bytes: usize,
    /// Largest permitted width or height, checked from the header before anything is allocated.
    pub max_dimension: u32,
    /// Largest permitted pixel count, likewise header-checked. This is the memory bound that
    /// actually bites: decoded RGBA is four bytes a pixel.
    pub max_pixels: u64,
    /// Hard cap on the PNG we emit. Enforced by the sink as it is written, so an animation that
    /// would inflate past this is abandoned rather than buffered to completion first.
    pub max_output_bytes: usize,
    /// Largest number of animation frames carried through to the APNG.
    pub max_frames: u32,
    /// Cumulative pixels across all retained frames. Frames must be collected before the APNG
    /// header can be written (`acTL` needs the count up front), so this is a live memory bound.
    pub max_animation_pixels: u64,
    /// Wall-clock ceiling. See [`DecodeBounds::max_duration`] notes on the module docs: this stops
    /// an animation loop mid-flight, but a single still decode inside `image` is not interruptible,
    /// so for stills it is only checked at stage boundaries.
    pub max_duration: Duration,
}

impl DecodeBounds {
    /// The ceilings the media scheme runs with.
    ///
    /// `max_dimension` is 16384 because that is the maximum texture edge Chromium will accept on
    /// Windows/D3D: anything wider could not be painted usefully even if we did decode it.
    ///
    /// `max_pixels` is 25 million, which covers a 24 MP camera frame and a 6K screenshot. The
    /// number comes from the memory arithmetic rather than from taste: a still costs roughly
    /// `4 * pixels` decoded plus up to `4 * pixels` encoded in flight, so 25 MP is about 200 MB
    /// transient worst case. That is why the integration plan also caps concurrent transcodes: the
    /// per-image bound is only a bound if the number of images in flight is bounded too.
    ///
    /// `max_input_bytes` is 32 MiB to match `MAX_WHOLE_IMAGE_BYTES` in `lib.rs`, the existing limit
    /// on how large a body the media handler will assemble in memory at all.
    ///
    /// `max_output_bytes` is 64 MiB, and it is picked to sit just above `max_pixels`. Measured on
    /// noisy photographic content (the worst case, because JPEG artefacts defeat PNG filtering),
    /// this encoder emits about 1.9 bytes a pixel, so a still at the 25 MP ceiling lands near
    /// 47 MB. A still can therefore never be refused by the output bound alone: anything that hits
    /// it is an animation, and an animation that will not fit 64 MiB is one nobody wants
    /// auto-loading. `still_output_fits_inside_the_pixel_bound` pins that relationship.
    ///
    /// `max_animation_pixels` is 24 million, so retained GIF frames cost at most ~96 MB. At a
    /// typical 480x360 that is roughly 130 frames; `max_frames` of 400 binds the small ones.
    ///
    /// `max_duration` is 3 seconds. An honest decode of anything inside the pixel bound finishes in
    /// well under a tenth of that, so the only things this catches are pathological.
    pub const DEFAULT: Self = Self {
        max_input_bytes: 32 * 1024 * 1024,
        max_dimension: 16_384,
        max_pixels: 25_000_000,
        max_output_bytes: 64 * 1024 * 1024,
        max_frames: 400,
        max_animation_pixels: 24_000_000,
        max_duration: Duration::from_secs(3),
    };

    /// Translate our ceilings into the decoder's own. This is belt and braces on purpose: we check
    /// the header ourselves, and we also tell `image` to refuse, so a decoder that allocates from a
    /// field we did not think to inspect still hits a wall.
    fn image_limits(&self) -> Limits {
        let mut limits = Limits::no_limits();
        limits.max_image_width = Some(self.max_dimension);
        limits.max_image_height = Some(self.max_dimension);
        // Four bytes a pixel for the output buffer, plus a fixed slack for decoder scratch (TIFF
        // strip buffers, the GIF canvas, the JPEG coefficient planes).
        limits.max_alloc = Some(self.max_pixels.saturating_mul(4).saturating_add(64 << 20));
        limits
    }
}

impl Default for DecodeBounds {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A transcoded image, ready to be the body of a media response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    /// The PNG (or APNG) bytes. Always `image/png`; see [`TRANSCODED_MIME`].
    pub png: Vec<u8>,
    /// Final width, after EXIF orientation has been applied.
    pub width: u32,
    /// Final height, after EXIF orientation has been applied.
    pub height: u32,
    /// Frames written. 1 for a still; more means the output is an APNG.
    pub frames: u32,
    /// Which decoder produced the pixels. For logs: "the GIF from this peer took 2.9s".
    pub source: SourceFormat,
    /// True when the source animation had more frames than [`DecodeBounds`] allowed and the output
    /// loops a prefix of it. Not an error, but a visible difference from what the sender saw.
    pub truncated_animation: bool,
}

impl DecodedImage {
    /// The MIME type to serve this under. Never the source type.
    #[must_use]
    pub fn mime(&self) -> &'static str {
        TRANSCODED_MIME
    }
}

/// Transcode one inline image with the default bounds.
///
/// `declared` is the MIME the manifest claims and `validated_inline_media_mime` already agreed
/// with; `bytes` is the whole file. The declaration chooses the decoder: this function never
/// sniffs, so bytes that do not match their declaration fail rather than being decoded as their
/// true format behind the validator's back.
pub fn transcode_inline_image(declared: &str, bytes: &[u8]) -> Result<DecodedImage, DecodeRefusal> {
    transcode_inline_image_with(declared, bytes, DecodeBounds::DEFAULT)
}

/// As [`transcode_inline_image`], with explicit bounds. Tests use this; production uses the
/// default. The unwind catch lives here rather than deeper so it covers header parsing, decode,
/// orientation and re-encode alike: every one of those runs on peer-controlled input.
pub fn transcode_inline_image_with(
    declared: &str,
    bytes: &[u8],
    bounds: DecodeBounds,
) -> Result<DecodedImage, DecodeRefusal> {
    let Some(format) = SourceFormat::from_mime(declared) else {
        return Err(DecodeRefusal::UnsupportedType);
    };
    if bytes.len() > bounds.max_input_bytes {
        return Err(DecodeRefusal::InputTooLarge);
    }
    // A pure-Rust decoder cannot corrupt memory, but it can absolutely panic: an index out of
    // range or a capacity overflow in `image`, `tiff` or `image-webp` is a bug reachable from a
    // peer's file. Containing the unwind turns that into the same click-to-load chip every other
    // refusal produces instead of killing the scheme handler's task.
    match catch_unwind(AssertUnwindSafe(|| transcode_inner(format, bytes, &bounds))) {
        Ok(result) => result,
        Err(_) => Err(DecodeRefusal::DecoderPanicked),
    }
}

fn transcode_inner(
    format: SourceFormat,
    bytes: &[u8],
    bounds: &DecodeBounds,
) -> Result<DecodedImage, DecodeRefusal> {
    let deadline = Instant::now() + bounds.max_duration;
    match format {
        SourceFormat::Gif => {
            let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(refusal_from_image)?;
            let (width, height) = decoder.dimensions();
            check_frame_bounds(width, height, bounds)?;
            transcode_frames(format, decoder.into_frames(), bounds, deadline)
        }
        SourceFormat::Webp => {
            let decoder = WebPDecoder::new(Cursor::new(bytes)).map_err(refusal_from_image)?;
            let (width, height) = decoder.dimensions();
            check_frame_bounds(width, height, bounds)?;
            if decoder.has_animation() {
                transcode_frames(format, decoder.into_frames(), bounds, deadline)
            } else {
                finish_still(
                    format,
                    decode_still(decoder, bounds, deadline)?,
                    bounds,
                    deadline,
                )
            }
        }
        SourceFormat::Png => {
            let decoder = PngDecoder::new(Cursor::new(bytes)).map_err(refusal_from_image)?;
            finish_still(
                format,
                decode_still(decoder, bounds, deadline)?,
                bounds,
                deadline,
            )
        }
        SourceFormat::Jpeg => {
            let decoder = JpegDecoder::new(Cursor::new(bytes)).map_err(refusal_from_image)?;
            finish_still(
                format,
                decode_still(decoder, bounds, deadline)?,
                bounds,
                deadline,
            )
        }
        SourceFormat::Bmp => {
            let decoder = BmpDecoder::new(Cursor::new(bytes)).map_err(refusal_from_image)?;
            finish_still(
                format,
                decode_still(decoder, bounds, deadline)?,
                bounds,
                deadline,
            )
        }
        SourceFormat::Tiff => {
            let decoder = TiffDecoder::new(Cursor::new(bytes)).map_err(refusal_from_image)?;
            finish_still(
                format,
                decode_still(decoder, bounds, deadline)?,
                bounds,
                deadline,
            )
        }
        SourceFormat::Ico => {
            let decoder = IcoDecoder::new(Cursor::new(bytes)).map_err(refusal_from_image)?;
            finish_still(
                format,
                decode_still(decoder, bounds, deadline)?,
                bounds,
                deadline,
            )
        }
    }
}

/// Decode one still frame under the bounds.
///
/// Order matters. The dimensions come from the header, before a single pixel buffer is allocated,
/// which is the only bound that is genuinely preventive: a PNG whose IHDR claims 30000x30000 costs
/// us the header parse and nothing else. Everything after that is paid for before it is checked.
fn decode_still<D: ImageDecoder>(
    mut decoder: D,
    bounds: &DecodeBounds,
    deadline: Instant,
) -> Result<DynamicImage, DecodeRefusal> {
    let (width, height) = decoder.dimensions();
    check_frame_bounds(width, height, bounds)?;
    decoder
        .set_limits(bounds.image_limits())
        .map_err(refusal_from_image)?;
    // WebView2 applies EXIF orientation to an `<img>` itself, so if we strip the metadata (and we
    // do: the output carries no ancillary chunks at all) without baking the rotation in, every
    // portrait phone photo in the app would suddenly render on its side. Failure to read the tag
    // is not fatal; an unreadable orientation is the same as no orientation.
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    if Instant::now() > deadline {
        return Err(DecodeRefusal::DeadlineExceeded);
    }
    let mut image = DynamicImage::from_decoder(decoder).map_err(refusal_from_image)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Re-encode a decoded still and assemble the result.
fn finish_still(
    format: SourceFormat,
    image: DynamicImage,
    bounds: &DecodeBounds,
    deadline: Instant,
) -> Result<DecodedImage, DecodeRefusal> {
    // Checked after the fact on purpose, and it does not undo the cost already paid. What it does
    // buy is that a decode which took absurdly long does not then also get an encode spent on it,
    // and that the caller hears about it. The real backstop is the caller's own timeout.
    if Instant::now() > deadline {
        return Err(DecodeRefusal::DeadlineExceeded);
    }
    // Orientation can transpose the image, so re-check rather than trusting the header numbers.
    let (width, height) = (image.width(), image.height());
    check_frame_bounds(width, height, bounds)?;
    let (color, pixels) = flatten(image);
    let png = encode_png(
        width,
        height,
        color,
        &[RawFrame {
            pixels,
            delay_ms: 0,
        }],
        bounds,
    )?;
    Ok(DecodedImage {
        png,
        width,
        height,
        frames: 1,
        source: format,
        truncated_animation: false,
    })
}

/// Decode an animated container into an APNG.
///
/// Flattening animations to a still would be the simpler module, and a worse product: GIFs are a
/// large share of what gets posted, and "the picture appears exactly as it does today" includes
/// moving. APNG keeps that inside an ordinary `<img>`, so nothing on the frontend changes.
///
/// Frames must be buffered before anything is written, because APNG's `acTL` chunk carries the
/// frame count and it comes first. That is why `max_animation_pixels` is a memory bound and not
/// just an output bound, and why the loop stops at the budget rather than after the fact.
fn transcode_frames(
    format: SourceFormat,
    frames: Frames<'_>,
    bounds: &DecodeBounds,
    deadline: Instant,
) -> Result<DecodedImage, DecodeRefusal> {
    debug_assert!(format.may_animate());
    let mut collected: Vec<RawFrame> = Vec::new();
    let mut width = 0u32;
    let mut height = 0u32;
    let mut truncated = false;
    let mut pixel_budget = bounds.max_animation_pixels;

    for frame in frames {
        // A failure partway through is not the same as a failure at frame zero. A GIF whose tail is
        // corrupt still has a usable head, and a truncated animation beats a broken-image icon, so
        // stop and keep what decoded rather than discarding the lot.
        let Ok(frame) = frame else {
            if collected.is_empty() {
                return Err(DecodeRefusal::Malformed);
            }
            truncated = true;
            break;
        };
        if Instant::now() > deadline {
            if collected.is_empty() {
                return Err(DecodeRefusal::DeadlineExceeded);
            }
            truncated = true;
            break;
        }
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let buffer = frame.into_buffer();
        let (frame_width, frame_height) = (buffer.width(), buffer.height());
        check_frame_bounds(frame_width, frame_height, bounds)?;
        if collected.is_empty() {
            width = frame_width;
            height = frame_height;
        } else if frame_width != width || frame_height != height {
            // Both animation decoders composite onto a fixed canvas, so this should be impossible.
            // If it ever is not, APNG has no way to express it and a wrong-sized `write_image_data`
            // would be a panic, so stop here instead.
            truncated = true;
            break;
        }
        let cost = u64::from(frame_width) * u64::from(frame_height);
        if collected.len() as u32 >= bounds.max_frames || cost > pixel_budget {
            truncated = true;
            break;
        }
        pixel_budget -= cost;
        collected.push(RawFrame {
            pixels: buffer.into_raw(),
            delay_ms: frame_delay_ms(numerator, denominator),
        });
    }

    if collected.is_empty() {
        return Err(DecodeRefusal::Malformed);
    }

    // A single-frame GIF is a still, and writing it as a one-frame APNG would put `acTL`/`fcTL`
    // chunks in the output for no reason. Narrower output is the point of the exercise.
    let frames_written = collected.len() as u32;
    // Animation frames are always RGBA here: both decoders composite through an alpha canvas, and
    // GIF's transparent index needs it.
    let png = match encode_png(width, height, png::ColorType::Rgba, &collected, bounds) {
        Ok(png) => png,
        // An animation that inflates past the output cap degrades to its first frame rather than
        // vanishing. Still bounded: one frame cannot exceed `max_pixels * 4` before compression.
        Err(DecodeRefusal::OutputOverBound) if frames_written > 1 => {
            let first = collected.drain_first();
            let png = encode_png(width, height, png::ColorType::Rgba, &[first], bounds)?;
            return Ok(DecodedImage {
                png,
                width,
                height,
                frames: 1,
                source: format,
                truncated_animation: true,
            });
        }
        Err(other) => return Err(other),
    };
    Ok(DecodedImage {
        png,
        width,
        height,
        frames: frames_written,
        source: format,
        truncated_animation: truncated,
    })
}

/// Take the first frame out of a collected animation, dropping the rest.
trait DrainFirst {
    fn drain_first(self) -> RawFrame;
}

impl DrainFirst for Vec<RawFrame> {
    fn drain_first(mut self) -> RawFrame {
        self.truncate(1);
        self.pop().expect("caller checked the vec is non-empty")
    }
}

/// One frame's worth of tightly packed samples, in the output colour type's channel order.
#[derive(Debug)]
struct RawFrame {
    pixels: Vec<u8>,
    /// Milliseconds to display. APNG stores a rational; we always write `delay_ms / 1000`.
    delay_ms: u16,
}

/// Convert a frame delay from `image`'s rational milliseconds into whole milliseconds.
///
/// Clamped at both ends. A zero delay is legal in GIF and means "as fast as possible", which every
/// browser silently rewrites to 100ms; writing 100 ourselves makes the output say what it does. The
/// upper clamp is the u16 the APNG numerator is stored in.
fn frame_delay_ms(numerator: u32, denominator: u32) -> u16 {
    if denominator == 0 {
        return 100;
    }
    let ms = numerator / denominator;
    if ms == 0 {
        return 100;
    }
    ms.min(u32::from(u16::MAX)) as u16
}

/// Reduce a decoded image to one of exactly two output shapes.
///
/// Two, not seven. Grayscale, 16-bit, palette and float sources all land on 8-bit truecolour or
/// truecolour-alpha, so the platform decoder only ever sees the two most ordinary PNG colour types.
/// The cost is real and worth naming: 16-bit sources lose precision, and grayscale ones triple in
/// raw size before compression (deflate takes nearly all of that back).
fn flatten(image: DynamicImage) -> (png::ColorType, Vec<u8>) {
    if image.color().has_alpha() {
        (png::ColorType::Rgba, image.into_rgba8().into_raw())
    } else {
        (png::ColorType::Rgb, image.into_rgb8().into_raw())
    }
}

/// Write the output PNG.
///
/// This is the narrow profile the whole module exists to produce, and every line of it is a
/// deliberate omission: 8-bit depth (no 16-bit unpacking path), no interlacing (no Adam7 pass
/// reconstruction), truecolour only (no palette or transparency chunk), and not one ancillary
/// chunk, so no iCCP, no zTXt, no eXIf. What reaches WebView2's decoder is IHDR, IDAT, IEND, plus
/// acTL/fcTL/fdAT when the source was animated.
fn encode_png(
    width: u32,
    height: u32,
    color: png::ColorType,
    frames: &[RawFrame],
    bounds: &DecodeBounds,
) -> Result<Vec<u8>, DecodeRefusal> {
    let expected = u64::from(width) * u64::from(height) * u64::from(color.samples() as u32);
    if frames.iter().any(|f| f.pixels.len() as u64 != expected) {
        // `write_image_data` panics on a length mismatch, so refuse before it can.
        return Err(DecodeRefusal::EncodeFailed);
    }
    let sink = CappedSink::new(bounds.max_output_bytes);
    let animated = frames.len() > 1;
    let result = (|| -> Result<(), png::EncodingError> {
        let mut encoder = png::Encoder::new(sink.clone(), width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        // fdeflate rather than a higher level: this runs on every inline image in a scrollback, and
        // the bytes never leave the machine, so encode latency matters and ratio does not.
        encoder.set_deflate_compression(png::DeflateCompression::FdeflateUltraFast);
        // Adaptive filtering is chosen explicitly because `set_compression`'s fast preset would
        // turn filtering off, which roughly doubles the output on photographic content.
        encoder.set_filter(png::Filter::Adaptive);
        if animated {
            encoder.set_animated(frames.len() as u32, 0)?;
        }
        let mut writer = encoder.write_header()?;
        for frame in frames {
            if animated {
                writer.set_frame_delay(frame.delay_ms, 1000)?;
                // Every frame here is a full composited canvas, so each one replaces the last
                // outright. Saying so is what stops a viewer alpha-blending frames together.
                writer.set_blend_op(png::BlendOp::Source)?;
                writer.set_dispose_op(png::DisposeOp::None)?;
            }
            writer.write_image_data(&frame.pixels)?;
        }
        writer.finish()
    })();
    // The sink's refusal surfaces as an ordinary io error, so check the flag before believing the
    // error is about anything else.
    if sink.overflowed() {
        return Err(DecodeRefusal::OutputOverBound);
    }
    result.map_err(|_| DecodeRefusal::EncodeFailed)?;
    Ok(sink.take())
}

/// Reject a frame whose size is outside the bounds, before it costs anything.
fn check_frame_bounds(width: u32, height: u32, bounds: &DecodeBounds) -> Result<(), DecodeRefusal> {
    if width == 0 || height == 0 {
        return Err(DecodeRefusal::Malformed);
    }
    if width > bounds.max_dimension || height > bounds.max_dimension {
        return Err(DecodeRefusal::DimensionsOverBound);
    }
    if u64::from(width) * u64::from(height) > bounds.max_pixels {
        return Err(DecodeRefusal::PixelsOverBound);
    }
    Ok(())
}

/// Map a decoder error onto a refusal.
///
/// The distinction that earns its keep is `Limits` versus the rest: a limit refusal means we were
/// right to be suspicious, while a decoding error usually means the file is simply broken, and the
/// two want different attention when they show up in a log.
fn refusal_from_image(error: ImageError) -> DecodeRefusal {
    match error {
        ImageError::Limits(_) => DecodeRefusal::PixelsOverBound,
        ImageError::Unsupported(_) => DecodeRefusal::UnsupportedEncoding,
        ImageError::Decoding(_)
        | ImageError::Encoding(_)
        | ImageError::Parameter(_)
        | ImageError::IoError(_) => DecodeRefusal::Malformed,
    }
}

/// A `Write` sink that stops accepting bytes once `cap` is passed.
///
/// The cap has to live in the sink rather than in a length check afterwards: an APNG that would
/// come to 900 MB should never be assembled at all, and the encoder owns its writer, so the flag
/// is shared out rather than read back off it.
#[derive(Debug, Clone)]
struct CappedSink {
    out: Rc<RefCell<Vec<u8>>>,
    overflowed: Rc<Cell<bool>>,
    cap: usize,
}

impl CappedSink {
    fn new(cap: usize) -> Self {
        Self {
            out: Rc::new(RefCell::new(Vec::new())),
            overflowed: Rc::new(Cell::new(false)),
            cap,
        }
    }

    fn overflowed(&self) -> bool {
        self.overflowed.get()
    }

    fn take(self) -> Vec<u8> {
        std::mem::take(&mut self.out.borrow_mut())
    }
}

impl Write for CappedSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut out = self.out.borrow_mut();
        if out.len().saturating_add(buf.len()) > self.cap {
            self.overflowed.set(true);
            return Err(io::Error::other(
                "transcoded image exceeds the output bound",
            ));
        }
        out.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::bmp::BmpEncoder;
    use image::codecs::gif::GifEncoder;
    use image::codecs::ico::IcoEncoder;
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::PngEncoder;
    use image::codecs::tiff::TiffEncoder;
    use image::codecs::webp::WebPEncoder;
    use image::{ExtendedColorType, Frame, ImageEncoder, RgbaImage};
    use rand_chacha::ChaCha8Rng;
    use rand_core::{RngCore, SeedableRng};

    /// Every MIME the inline allowlist admits today, including the ones we refuse to transcode.
    const ALLOWLISTED_IMAGE_MIMES: &[&str] = &[
        "image/png",
        "image/jpeg",
        "image/gif",
        "image/webp",
        "image/avif",
        "image/bmp",
        "image/tiff",
        "image/x-icon",
    ];

    const TRANSCODABLE: &[SourceFormat] = &[
        SourceFormat::Png,
        SourceFormat::Jpeg,
        SourceFormat::Gif,
        SourceFormat::Webp,
        SourceFormat::Bmp,
        SourceFormat::Tiff,
        SourceFormat::Ico,
    ];

    /// A deterministic little test image: a gradient, so JPEG's lossy round trip still lands close
    /// to the source and a flat fill cannot hide a channel-order mistake.
    fn sample(width: u32, height: u32, opaque: bool) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            let alpha = if opaque {
                255
            } else {
                ((x * 8 + y * 4) % 256) as u8
            };
            image::Rgba([
                (x * 7 % 256) as u8,
                (y * 5 % 256) as u8,
                ((x + y) * 3 % 256) as u8,
                alpha,
            ])
        })
    }

    fn encode_fixture(format: SourceFormat, image: &RgbaImage) -> Vec<u8> {
        let (width, height) = (image.width(), image.height());
        let mut out = Vec::new();
        match format {
            SourceFormat::Png => PngEncoder::new(&mut out)
                .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
                .expect("png fixture encodes"),
            SourceFormat::Jpeg => {
                let rgb = DynamicImage::ImageRgba8(image.clone()).into_rgb8();
                JpegEncoder::new(&mut out)
                    .write_image(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                    .expect("jpeg fixture encodes");
            }
            SourceFormat::Gif => {
                let mut encoder = GifEncoder::new(&mut out);
                encoder
                    .encode(image.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .expect("gif fixture encodes");
            }
            SourceFormat::Webp => WebPEncoder::new_lossless(&mut out)
                .encode(image.as_raw(), width, height, ExtendedColorType::Rgba8)
                .expect("webp fixture encodes"),
            SourceFormat::Bmp => {
                let mut cursor = Cursor::new(&mut out);
                BmpEncoder::new(&mut cursor)
                    .encode(image.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .expect("bmp fixture encodes");
            }
            SourceFormat::Tiff => {
                let cursor = Cursor::new(&mut out);
                TiffEncoder::new(cursor)
                    .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
                    .expect("tiff fixture encodes");
            }
            SourceFormat::Ico => IcoEncoder::new(&mut out)
                .write_image(image.as_raw(), width, height, ExtendedColorType::Rgba8)
                .expect("ico fixture encodes"),
        }
        out
    }

    /// An animated GIF of `frames` frames, all the same canvas.
    fn animated_gif(frames: u32, width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = GifEncoder::new(&mut out);
            for index in 0..frames {
                let mut canvas = sample(width, height, true);
                for pixel in canvas.pixels_mut() {
                    pixel.0[0] = pixel.0[0].wrapping_add(index as u8 * 17);
                }
                encoder
                    .encode_frame(Frame::new(canvas))
                    .expect("animated gif fixture encodes");
            }
        }
        out
    }

    /// Read our own output back, so assertions are about the bytes we emit rather than about the
    /// struct we filled in. Returns the parsed PNG info plus the frame count.
    fn reread(png_bytes: &[u8]) -> (png::OutputInfo, png::Info<'static>) {
        let decoder = png::Decoder::new(Cursor::new(png_bytes));
        let mut reader = decoder.read_info().expect("our own png parses");
        let mut buffer = vec![0; reader.output_buffer_size().expect("bounded output size")];
        let info = reader.next_frame(&mut buffer).expect("our own png decodes");
        (info, reader.info().clone())
    }

    #[test]
    fn every_admitted_format_decodes_to_a_png() {
        for &format in TRANSCODABLE {
            // ICO stores an 8-bit side length, so 64 is the one size every format can carry.
            let source = sample(64, 48, true);
            let source = if format == SourceFormat::Ico {
                sample(64, 64, true)
            } else {
                source
            };
            let bytes = encode_fixture(format, &source);
            let decoded = transcode_inline_image(format.as_mime(), &bytes)
                .unwrap_or_else(|e| panic!("{} should transcode, got {e}", format.as_mime()));
            assert_eq!(decoded.source, format);
            assert_eq!(decoded.mime(), "image/png");
            assert_eq!(decoded.frames, 1, "{format:?} is a still");
            assert_eq!((decoded.width, decoded.height), source.dimensions());
            assert!(
                decoded.png.starts_with(b"\x89PNG\r\n\x1a\n"),
                "{format:?} output is a PNG"
            );
            let (info, _) = reread(&decoded.png);
            assert_eq!(info.width, source.width());
            assert_eq!(info.height, source.height());
        }
    }

    /// The narrow profile is the security claim, so it is asserted rather than assumed.
    #[test]
    fn output_uses_only_the_narrow_profile() {
        for &format in TRANSCODABLE {
            let size = if format == SourceFormat::Ico { 64 } else { 32 };
            for opaque in [true, false] {
                let source = sample(size, size, opaque);
                let bytes = encode_fixture(format, &source);
                let Ok(decoded) = transcode_inline_image(format.as_mime(), &bytes) else {
                    panic!("{format:?} opaque={opaque} should transcode");
                };
                let (info, full) = reread(&decoded.png);
                assert_eq!(info.bit_depth, png::BitDepth::Eight, "{format:?}");
                assert!(
                    matches!(info.color_type, png::ColorType::Rgb | png::ColorType::Rgba),
                    "{format:?} emitted {:?}",
                    info.color_type
                );
                assert!(!full.interlaced, "{format:?} must not interlace");
                assert!(full.palette.is_none(), "{format:?} must not emit a palette");
                assert!(full.icc_profile.is_none(), "{format:?} must not emit iCCP");
                assert!(
                    full.uncompressed_latin1_text.is_empty()
                        && full.compressed_latin1_text.is_empty()
                        && full.utf8_text.is_empty(),
                    "{format:?} must not emit text chunks"
                );
                assert!(
                    full.animation_control.is_none(),
                    "{format:?} still must not claim to be animated"
                );
            }
        }
    }

    #[test]
    fn alpha_survives_and_opaque_sources_stay_rgb() {
        let transparent = sample(16, 16, false);
        let bytes = encode_fixture(SourceFormat::Png, &transparent);
        let decoded = transcode_inline_image("image/png", &bytes).expect("transcodes");
        let (info, _) = reread(&decoded.png);
        assert_eq!(info.color_type, png::ColorType::Rgba);

        // A JPEG has no alpha channel at all, so the output must not invent one.
        let opaque = sample(16, 16, true);
        let bytes = encode_fixture(SourceFormat::Jpeg, &opaque);
        let decoded = transcode_inline_image("image/jpeg", &bytes).expect("transcodes");
        let (info, _) = reread(&decoded.png);
        assert_eq!(info.color_type, png::ColorType::Rgb);
    }

    #[test]
    fn pixels_survive_a_lossless_round_trip() {
        // PNG in, PNG out, no lossy stage anywhere: the samples must be identical, or the module
        // is silently altering what members see.
        let source = sample(24, 17, false);
        let bytes = encode_fixture(SourceFormat::Png, &source);
        let decoded = transcode_inline_image("image/png", &bytes).expect("transcodes");
        let png_decoder = png::Decoder::new(Cursor::new(&decoded.png));
        let mut reader = png_decoder.read_info().expect("parses");
        let mut buffer = vec![0; reader.output_buffer_size().expect("bounded")];
        let info = reader.next_frame(&mut buffer).expect("decodes");
        assert_eq!(&buffer[..info.buffer_size()], source.as_raw().as_slice());
    }

    #[test]
    fn transcoding_is_deterministic() {
        let source = sample(40, 40, false);
        let bytes = encode_fixture(SourceFormat::Png, &source);
        let first = transcode_inline_image("image/png", &bytes).expect("transcodes");
        let second = transcode_inline_image("image/png", &bytes).expect("transcodes");
        assert_eq!(first, second, "a cached response must not depend on timing");
    }

    #[test]
    fn avif_is_refused_because_it_has_no_pure_rust_decoder() {
        // The only AVIF decoders available are bindings to C. Refusing here is the whole reason
        // this module can claim anything: an AVIF that reached the webview would be exactly the
        // attack we are closing.
        assert_eq!(SourceFormat::from_mime("image/avif"), None);
        let mut avif = Vec::from(&b"\0\0\0\x20ftypavif"[..]);
        avif.extend_from_slice(&[0u8; 32]);
        assert_eq!(
            transcode_inline_image("image/avif", &avif),
            Err(DecodeRefusal::UnsupportedType)
        );
    }

    #[test]
    fn non_image_and_unknown_declarations_are_refused() {
        let png = encode_fixture(SourceFormat::Png, &sample(8, 8, true));
        for declared in [
            "image/svg+xml",
            "text/html",
            "application/octet-stream",
            "audio/mpeg",
            "video/mp4",
            "video/webm",
            "image/heic",
            "image/",
            "",
            "png",
        ] {
            assert_eq!(
                transcode_inline_image(declared, &png),
                Err(DecodeRefusal::UnsupportedType),
                "{declared} must not transcode"
            );
        }
    }

    /// The allowlist in `lib.rs` and the transcodable set here have to be reconcilable, and the
    /// only difference may be AVIF. If another image type is ever admitted, this fails and whoever
    /// added it has to decide which side it belongs on.
    #[test]
    fn the_allowlist_splits_exactly_into_transcodable_and_avif() {
        for mime in ALLOWLISTED_IMAGE_MIMES {
            let decodable = SourceFormat::from_mime(mime).is_some();
            assert_eq!(
                decodable,
                *mime != "image/avif",
                "{mime} is on the wrong side of the split"
            );
        }
        for format in TRANSCODABLE {
            assert!(ALLOWLISTED_IMAGE_MIMES.contains(&format.as_mime()));
        }
    }

    #[test]
    fn mime_parameters_and_case_are_normalised() {
        let png = encode_fixture(SourceFormat::Png, &sample(8, 8, true));
        for declared in ["image/png", "IMAGE/PNG", "  image/png  ", "image/png; q=1"] {
            assert!(
                transcode_inline_image(declared, &png).is_ok(),
                "{declared} should normalise to image/png"
            );
        }
    }

    /// The declaration picks the decoder. Nothing here sniffs, so a body that disagrees with its
    /// declaration is refused rather than decoded as its true format behind the validator's back.
    #[test]
    fn the_declaration_chooses_the_decoder_and_never_the_bytes() {
        let png = encode_fixture(SourceFormat::Png, &sample(16, 16, true));
        for declared in ["image/jpeg", "image/gif", "image/bmp", "image/tiff"] {
            let outcome = transcode_inline_image(declared, &png);
            assert!(
                outcome.is_err(),
                "a PNG declared {declared} must not transcode, got {outcome:?}"
            );
        }
    }

    /// A hand-built PNG header claiming an enormous canvas, with no image data behind it. This is
    /// the decompression-bomb shape: a few dozen bytes that would cost gigabytes to honour. The
    /// refusal has to happen at the header, before anything is allocated.
    fn oversized_png_header(width: u32, height: u32) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = 0xffff_ffffu32;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        (crc >> 1) ^ 0xedb8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], body: &[u8]) {
            out.extend_from_slice(&(body.len() as u32).to_be_bytes());
            let mut crc_input = Vec::from(&kind[..]);
            crc_input.extend_from_slice(body);
            out.extend_from_slice(kind);
            out.extend_from_slice(body);
            out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        }
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, deflate, adaptive filter, no interlace
        let mut out = Vec::from(&b"\x89PNG\r\n\x1a\n"[..]);
        chunk(&mut out, b"IHDR", &ihdr);
        // A well-formed but empty zlib stream: a final stored block of zero length. The file has to
        // get as far as having image data for the header parse to succeed, and the whole point of
        // the fixture is that the refusal happens before anyone asks what the data decompresses to.
        chunk(
            &mut out,
            b"IDAT",
            &[
                0x78, 0x01, 0x01, 0x00, 0x00, 0xff, 0xff, 0x00, 0x00, 0x00, 0x01,
            ],
        );
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn a_bomb_is_refused_at_the_header_not_after_allocating() {
        // 30000 x 30000 RGBA is 3.6 GB. Sixty-odd bytes of input.
        let bomb = oversized_png_header(30_000, 30_000);
        assert!(bomb.len() < 100, "the bomb is small, that is the point");
        assert_eq!(
            transcode_inline_image("image/png", &bomb),
            Err(DecodeRefusal::DimensionsOverBound)
        );
        // Inside the per-side cap but far past the pixel cap: 16000 x 16000 is 256 megapixels.
        assert_eq!(
            transcode_inline_image("image/png", &oversized_png_header(16_000, 16_000)),
            Err(DecodeRefusal::PixelsOverBound)
        );
        // A zero side is malformed, not oversized.
        assert_eq!(
            transcode_inline_image("image/png", &oversized_png_header(0, 10)),
            Err(DecodeRefusal::Malformed)
        );
    }

    #[test]
    fn default_bounds_are_the_documented_numbers() {
        // These are load-bearing for the memory arithmetic in the module docs, so a change to any
        // of them should be a deliberate edit here too.
        let bounds = DecodeBounds::DEFAULT;
        assert_eq!(bounds.max_input_bytes, 32 * 1024 * 1024);
        assert_eq!(bounds.max_dimension, 16_384);
        assert_eq!(bounds.max_pixels, 25_000_000);
        assert_eq!(bounds.max_output_bytes, 64 * 1024 * 1024);
        assert_eq!(bounds.max_frames, 400);
        assert_eq!(bounds.max_animation_pixels, 24_000_000);
        assert_eq!(bounds.max_duration, Duration::from_secs(3));
        assert_eq!(DecodeBounds::default(), bounds);
        // Worst case transient for one still: decoded RGBA plus an incompressible output.
        assert!(bounds.max_pixels * 4 + bounds.max_output_bytes as u64 <= 256 << 20);
    }

    /// The pixel bound and the output bound have to agree, or one of them is decoration. At the
    /// measured worst case of roughly two bytes a pixel, a still at the pixel ceiling must still
    /// fit the output ceiling: otherwise large photographs would refuse for a reason that reads as
    /// "too big to send" when the real cause is an encoder ratio nobody wrote down.
    #[test]
    fn still_output_fits_inside_the_pixel_bound() {
        let bounds = DecodeBounds::DEFAULT;
        let worst_case_bytes = bounds.max_pixels * 2;
        assert!(
            worst_case_bytes < bounds.max_output_bytes as u64,
            "a {} MP still at ~2 bytes/px is {} MiB, over the {} MiB output cap",
            bounds.max_pixels / 1_000_000,
            worst_case_bytes >> 20,
            bounds.max_output_bytes >> 20
        );
    }

    #[test]
    fn each_bound_refuses_with_its_own_reason() {
        let source = sample(32, 32, true);
        let png = encode_fixture(SourceFormat::Png, &source);

        let tight_input = DecodeBounds {
            max_input_bytes: png.len() - 1,
            ..DecodeBounds::DEFAULT
        };
        assert_eq!(
            transcode_inline_image_with("image/png", &png, tight_input),
            Err(DecodeRefusal::InputTooLarge)
        );

        let tight_dimension = DecodeBounds {
            max_dimension: 31,
            ..DecodeBounds::DEFAULT
        };
        assert_eq!(
            transcode_inline_image_with("image/png", &png, tight_dimension),
            Err(DecodeRefusal::DimensionsOverBound)
        );

        let tight_pixels = DecodeBounds {
            max_pixels: 32 * 32 - 1,
            ..DecodeBounds::DEFAULT
        };
        assert_eq!(
            transcode_inline_image_with("image/png", &png, tight_pixels),
            Err(DecodeRefusal::PixelsOverBound)
        );

        let tight_output = DecodeBounds {
            max_output_bytes: 16,
            ..DecodeBounds::DEFAULT
        };
        assert_eq!(
            transcode_inline_image_with("image/png", &png, tight_output),
            Err(DecodeRefusal::OutputOverBound)
        );

        let no_time = DecodeBounds {
            max_duration: Duration::ZERO,
            ..DecodeBounds::DEFAULT
        };
        assert_eq!(
            transcode_inline_image_with("image/png", &png, no_time),
            Err(DecodeRefusal::DeadlineExceeded)
        );

        // And the bounds that should not fire do not.
        assert!(transcode_inline_image_with("image/png", &png, DecodeBounds::DEFAULT).is_ok());
    }

    #[test]
    fn an_animated_gif_becomes_an_apng() {
        let gif = animated_gif(5, 24, 24);
        let decoded = transcode_inline_image("image/gif", &gif).expect("animated gif transcodes");
        assert_eq!(decoded.frames, 5);
        assert!(!decoded.truncated_animation);
        let (_, info) = reread(&decoded.png);
        let control = info
            .animation_control
            .expect("an animated source must produce acTL");
        assert_eq!(control.num_frames, 5);
        assert_eq!(control.num_plays, 0, "loop forever, as a GIF does");
        assert_eq!(info.color_type, png::ColorType::Rgba);
    }

    #[test]
    fn an_animation_over_budget_is_truncated_not_refused() {
        let gif = animated_gif(6, 16, 16);
        let few_frames = DecodeBounds {
            max_frames: 2,
            ..DecodeBounds::DEFAULT
        };
        let decoded =
            transcode_inline_image_with("image/gif", &gif, few_frames).expect("still transcodes");
        assert_eq!(decoded.frames, 2);
        assert!(decoded.truncated_animation);

        // The cumulative pixel budget binds independently of the frame count.
        let few_pixels = DecodeBounds {
            max_animation_pixels: 16 * 16 * 3,
            ..DecodeBounds::DEFAULT
        };
        let decoded =
            transcode_inline_image_with("image/gif", &gif, few_pixels).expect("still transcodes");
        assert_eq!(decoded.frames, 3);
        assert!(decoded.truncated_animation);
    }

    #[test]
    fn an_animation_that_will_not_fit_the_output_bound_degrades_to_a_still() {
        let gif = animated_gif(8, 32, 32);
        // Enough room for one frame, nowhere near enough for eight.
        let squeezed = DecodeBounds {
            max_output_bytes: 5_000,
            ..DecodeBounds::DEFAULT
        };
        let decoded =
            transcode_inline_image_with("image/gif", &gif, squeezed).expect("degrades to a still");
        assert_eq!(decoded.frames, 1);
        assert!(decoded.truncated_animation);
        let (_, info) = reread(&decoded.png);
        assert!(info.animation_control.is_none());
    }

    #[test]
    fn a_single_frame_gif_stays_a_still() {
        let gif = encode_fixture(SourceFormat::Gif, &sample(16, 16, true));
        let decoded = transcode_inline_image("image/gif", &gif).expect("transcodes");
        assert_eq!(decoded.frames, 1);
        let (_, info) = reread(&decoded.png);
        assert!(
            info.animation_control.is_none(),
            "a one-frame source must not carry animation chunks"
        );
    }

    #[test]
    fn truncated_files_fail_cleanly_for_every_format() {
        for &format in TRANSCODABLE {
            let size = if format == SourceFormat::Ico { 64 } else { 48 };
            let bytes = encode_fixture(format, &sample(size, size, true));
            // A spread of cut points, because where a format breaks depends on where its header,
            // its index and its payload sit.
            for numerator in [1usize, 2, 3, 5, 7, 9] {
                let cut = bytes.len() * numerator / 10;
                let truncated = &bytes[..cut];
                match transcode_inline_image(format.as_mime(), truncated) {
                    Err(DecodeRefusal::DecoderPanicked) => {
                        panic!("{format:?} truncated to {cut} bytes panicked its decoder")
                    }
                    // Some formats can still produce a complete image from a prefix (a BMP whose
                    // pixel array is simply short, an animation whose first frame is intact). That
                    // is fine: the point is that it is a decided outcome, not a crash, and that
                    // anything that does come out still obeys the bounds.
                    Ok(image) => {
                        assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"));
                        assert!(image.width <= DecodeBounds::DEFAULT.max_dimension);
                        assert!(image.height <= DecodeBounds::DEFAULT.max_dimension);
                    }
                    Err(_) => {}
                }
            }
        }
    }

    #[test]
    fn empty_and_tiny_inputs_are_refused_without_panicking() {
        for &format in TRANSCODABLE {
            for length in [0usize, 1, 2, 3, 8, 16] {
                let bytes = vec![0u8; length];
                assert_ne!(
                    transcode_inline_image(format.as_mime(), &bytes).err(),
                    Some(DecodeRefusal::DecoderPanicked),
                    "{format:?} panicked on {length} zero bytes"
                );
            }
        }
    }

    /// `proptest` is a dev-dependency of the root workspace, not of this crate, and the brief was
    /// not to add one. This is the same idea with the dependency we already have: a fixed seed, so
    /// a failure is reproducible, feeding both pure noise and mutated valid files through every
    /// decoder. The assertion is the weak one that matters: a peer's file never unwinds past us.
    #[test]
    fn arbitrary_input_never_escapes_as_a_panic() {
        let mut rng = ChaCha8Rng::seed_from_u64(0x6d65_7774_7561_6c00);
        let mut panics = Vec::new();
        let mut decoded = 0usize;

        for &format in TRANSCODABLE {
            let size = if format == SourceFormat::Ico { 64 } else { 32 };
            let valid = encode_fixture(format, &sample(size, size, false));

            for case in 0..96 {
                let bytes = if case % 3 == 0 {
                    // Pure noise, sometimes with a plausible magic prefix so the header parser is
                    // actually entered rather than bailing on the first byte.
                    let length = 16 + (rng.next_u32() as usize % 4096);
                    let mut bytes = vec![0u8; length];
                    rng.fill_bytes(&mut bytes);
                    if case % 6 == 0 {
                        let prefix = valid.len().min(12);
                        bytes[..prefix].copy_from_slice(&valid[..prefix]);
                    }
                    bytes
                } else if case % 3 == 1 {
                    // A valid file with a handful of bytes corrupted: the shape most likely to get
                    // deep into a decoder before anything looks wrong.
                    let mut bytes = valid.clone();
                    let flips = 1 + (rng.next_u32() as usize % 8);
                    for _ in 0..flips {
                        let at = rng.next_u32() as usize % bytes.len();
                        bytes[at] ^= 1 << (rng.next_u32() % 8);
                    }
                    bytes
                } else {
                    // A valid file cut at an arbitrary point, then extended with noise, so lengths
                    // and declared offsets disagree.
                    let cut = 1 + (rng.next_u32() as usize % bytes_len(&valid));
                    let mut bytes = valid[..cut].to_vec();
                    let tail = rng.next_u32() as usize % 64;
                    let mut noise = vec![0u8; tail];
                    rng.fill_bytes(&mut noise);
                    bytes.extend_from_slice(&noise);
                    bytes
                };

                match transcode_inline_image(format.as_mime(), &bytes) {
                    Ok(image) => {
                        decoded += 1;
                        // Anything that does come out still has to obey the bounds: a decoder that
                        // returns a 40000-wide image from a corrupt header must not reach the
                        // encoder, let alone the webview.
                        assert!(image.width <= DecodeBounds::DEFAULT.max_dimension);
                        assert!(image.height <= DecodeBounds::DEFAULT.max_dimension);
                        assert!(
                            u64::from(image.width) * u64::from(image.height)
                                <= DecodeBounds::DEFAULT.max_pixels
                        );
                        assert!(image.png.len() <= DecodeBounds::DEFAULT.max_output_bytes);
                        assert!(image.png.starts_with(b"\x89PNG\r\n\x1a\n"));
                    }
                    Err(DecodeRefusal::DecoderPanicked) => panics.push((format, case)),
                    Err(_) => {}
                }
            }
        }

        // Containment is the guarantee; a contained panic is still a dependency bug worth naming,
        // so the seed is fixed and this list is expected to stay empty.
        assert!(
            panics.is_empty(),
            "decoders panicked on mutated input (contained, but real): {panics:?}"
        );
        assert!(
            decoded > 0,
            "the corpus never once decoded, so this proved nothing about the success path"
        );
    }

    fn bytes_len(bytes: &[u8]) -> usize {
        bytes.len().max(1)
    }

    #[test]
    fn refusals_have_distinct_stable_tokens() {
        let all = [
            DecodeRefusal::UnsupportedType,
            DecodeRefusal::InputTooLarge,
            DecodeRefusal::DimensionsOverBound,
            DecodeRefusal::PixelsOverBound,
            DecodeRefusal::UnsupportedEncoding,
            DecodeRefusal::Malformed,
            DecodeRefusal::DeadlineExceeded,
            DecodeRefusal::OutputOverBound,
            DecodeRefusal::EncodeFailed,
            DecodeRefusal::DecoderPanicked,
        ];
        let mut seen = Vec::new();
        for refusal in all {
            assert_eq!(refusal.to_string(), refusal.as_str());
            assert!(!refusal.as_str().is_empty());
            assert!(!seen.contains(&refusal.as_str()), "{refusal} is not unique");
            seen.push(refusal.as_str());
        }
    }

    #[test]
    fn the_capped_sink_stops_at_its_cap() {
        let mut sink = CappedSink::new(8);
        assert!(sink.write_all(&[0u8; 8]).is_ok());
        assert!(!sink.overflowed());
        assert!(sink.write_all(&[0u8; 1]).is_err());
        assert!(sink.overflowed());
        assert_eq!(
            sink.clone().take().len(),
            8,
            "nothing past the cap was kept"
        );
    }

    #[test]
    fn frame_delays_are_clamped_to_something_a_viewer_will_honour() {
        assert_eq!(
            frame_delay_ms(0, 1),
            100,
            "a zero delay means 100ms in practice"
        );
        assert_eq!(
            frame_delay_ms(50, 0),
            100,
            "a zero denominator is not a divide"
        );
        assert_eq!(frame_delay_ms(40, 1), 40);
        assert_eq!(frame_delay_ms(1_000_000, 1), u16::MAX);
    }
}
