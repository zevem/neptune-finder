//! Bounded file previews. Reading, image decoding and resampling never run in a frame.

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::fs::{self, File};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

const MAX_TEXT_BYTES: usize = 128 * 1024;
const MAX_IMAGE_INPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 16_000_000;
const MAX_IMAGE_DECODE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PREVIEW_WIDTH: u32 = 1_600;
const MAX_PREVIEW_HEIGHT: u32 = 1_200;
const TRUNCATION_NOTICE: &str = "\n\n… Preview limited to 128 KiB.";

#[derive(Debug)]
pub enum PreviewContent {
    Text(String),
    Image { size: [usize; 2], rgba: Vec<u8> },
    Unsupported(String),
}

#[derive(Debug)]
pub struct PreviewResult {
    pub generation: u64,
    pub path: PathBuf,
    pub result: Result<PreviewContent, String>,
}

struct Request {
    generation: u64,
    path: PathBuf,
}

#[derive(Default)]
struct State {
    pending: Option<Request>,
    completed: Option<PreviewResult>,
    closed: bool,
    startup_error: Option<String>,
}

struct Shared {
    generation: AtomicU64,
    state: Mutex<State>,
    changed: Condvar,
    wake: Arc<dyn Fn() + Send + Sync>,
}

/// A sleeping worker with one coalescing request slot and one result slot.
/// Requests cancel older reads; decoders are bounded and stale pixels are never
/// published. Drop requests shutdown without waiting on a slow filesystem call.
pub struct Previews {
    shared: Arc<Shared>,
}

impl Previews {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            generation: AtomicU64::new(0),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            wake: Arc::new(wake),
        });
        let worker = Arc::clone(&shared);
        if let Err(error) = std::thread::Builder::new()
            .name("neptune-finder-preview".into())
            .spawn(move || run_worker(worker))
        {
            lock_state(&shared).startup_error =
                Some(format!("Could not start the preview worker: {error}"));
        }
        Self { shared }
    }

    pub fn request(&self, path: PathBuf) -> u64 {
        let mut state = lock_state(&self.shared);
        let generation = self
            .shared
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        if let Some(error) = state.startup_error.clone() {
            state.completed = Some(PreviewResult {
                generation,
                path,
                result: Err(error),
            });
            drop(state);
            (self.shared.wake)();
        } else {
            state.pending = Some(Request { generation, path });
            drop(state);
            self.shared.changed.notify_one();
        }
        generation
    }

    pub fn poll(&self) -> Option<PreviewResult> {
        let mut state = lock_state(&self.shared);
        if state.completed.as_ref()?.generation != self.shared.generation.load(Ordering::Acquire) {
            // Let the worker release stale image buffers outside its queue lock.
            return None;
        }
        state.completed.take()
    }
}

impl Drop for Previews {
    fn drop(&mut self) {
        let mut state = lock_state(&self.shared);
        state.closed = true;
        state.pending = None;
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        drop(state);
        self.shared.changed.notify_one();
    }
}

fn lock_state(shared: &Shared) -> MutexGuard<'_, State> {
    shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn run_worker(shared: Arc<Shared>) {
    loop {
        let request = {
            let mut state = lock_state(&shared);
            while state.pending.is_none() && !state.closed {
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed {
                // Clear pixels here even if the UI still owns the last Arc
                // while finishing Previews::drop.
                let completed = state.completed.take();
                let pending = state.pending.take();
                drop(state);
                drop(completed);
                drop(pending);
                return;
            }
            state.pending.take().expect("pending preview request")
        };
        let cancelled = || shared.generation.load(Ordering::Acquire) != request.generation;
        let Some(result) = read_preview(&request.path, cancelled) else {
            continue;
        };
        let mut state = lock_state(&shared);
        if state.closed || cancelled() {
            continue;
        }
        let previous = state.completed.replace(PreviewResult {
            generation: request.generation,
            path: request.path,
            result,
        });
        drop(state);
        drop(previous);
        (shared.wake)();
    }
}

fn read_preview(
    path: &Path,
    cancelled: impl Fn() -> bool,
) -> Option<Result<PreviewContent, String>> {
    if cancelled() {
        return None;
    }
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => return Some(Err(format!("Could not inspect this item: {error}"))),
    };
    // Do not open devices, sockets or FIFOs: even a read of one byte can wait.
    if !metadata.is_file() {
        return Some(Ok(PreviewContent::Unsupported(if metadata.is_dir() {
            "Select a file to preview its contents.".into()
        } else {
            "This item does not have a file preview.".into()
        })));
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) => return Some(Err(format!("Could not open this file: {error}"))),
    };
    let prefix = match read_bounded(&mut file, MAX_TEXT_BYTES, &cancelled)? {
        Ok(prefix) => prefix,
        Err(error) => return Some(Err(error)),
    };
    if cancelled() {
        return None;
    }
    let format = image::guess_format(&prefix.bytes).ok();
    if let Some(format) = format {
        if !matches!(
            format,
            ImageFormat::Png
                | ImageFormat::Jpeg
                | ImageFormat::Gif
                | ImageFormat::WebP
                | ImageFormat::Bmp
        ) {
            return Some(Ok(PreviewContent::Unsupported(
                "Preview supports PNG, JPEG, GIF, WebP and BMP images.".into(),
            )));
        }
        if metadata.len() > MAX_IMAGE_INPUT_BYTES as u64 {
            return Some(Ok(PreviewContent::Unsupported(
                "This image exceeds the 32 MiB preview input limit.".into(),
            )));
        }
        // The first read only allocated a text-sized buffer. Rewind before the
        // bounded image read, including when the file has grown since metadata.
        if let Err(error) = file.seek(SeekFrom::Start(0)) {
            return Some(Err(format!("Could not read this image: {error}")));
        }
        let input = match read_bounded(&mut file, MAX_IMAGE_INPUT_BYTES, &cancelled)? {
            Ok(input) => input,
            Err(error) => return Some(Err(error)),
        };
        if input.truncated {
            return Some(Ok(PreviewContent::Unsupported(
                "This image exceeds the 32 MiB preview input limit.".into(),
            )));
        }
        drop(prefix);
        decode_image(&input.bytes, format, cancelled)
    } else {
        Some(Ok(decode_text(prefix)))
    }
}

struct BoundedBytes {
    bytes: Vec<u8>,
    truncated: bool,
}

/// The buffer never grows beyond `limit`. A one-byte stack probe distinguishes
/// exact-limit files from longer files without allocating an overflow byte.
fn read_bounded(
    file: &mut impl Read,
    limit: usize,
    cancelled: &impl Fn() -> bool,
) -> Option<Result<BoundedBytes, String>> {
    let mut bytes = Vec::with_capacity(limit.min(32 * 1024));
    let mut chunk = [0_u8; 16 * 1024];
    while bytes.len() < limit {
        if cancelled() {
            return None;
        }
        let wanted = chunk.len().min(limit - bytes.len());
        match file.read(&mut chunk[..wanted]) {
            Ok(0) => {
                return Some(Ok(BoundedBytes {
                    bytes,
                    truncated: false,
                }));
            }
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Some(Err(format!("Could not read this file: {error}"))),
        }
    }
    let mut probe = [0_u8; 1];
    loop {
        if cancelled() {
            return None;
        }
        match file.read(&mut probe) {
            Ok(count) => {
                return Some(Ok(BoundedBytes {
                    bytes,
                    truncated: count != 0,
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Some(Err(format!("Could not read this file: {error}"))),
        }
    }
}

fn decode_text(mut input: BoundedBytes) -> PreviewContent {
    let binary = input.bytes.iter().any(|byte| {
        (*byte < 0x20 && !matches!(*byte, b'\n' | b'\r' | b'\t' | 0x0c)) || *byte == 0x7f
    });
    if binary {
        return PreviewContent::Unsupported("Binary files do not have a text preview.".into());
    }
    match std::str::from_utf8(&input.bytes) {
        Ok(_) => {}
        Err(error) if input.truncated && error.error_len().is_none() => {
            // A valid multi-byte character may straddle the read boundary.
            input.bytes.truncate(error.valid_up_to());
        }
        Err(_) => {
            return PreviewContent::Unsupported(
                "This file is binary or uses an unsupported text encoding.".into(),
            );
        }
    }
    let mut text = String::from_utf8(input.bytes).expect("validated preview UTF-8");
    if input.truncated {
        let mut end = text.len().min(MAX_TEXT_BYTES - TRUNCATION_NOTICE.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push_str(TRUNCATION_NOTICE);
    }
    PreviewContent::Text(text)
}

fn decode_image(
    bytes: &[u8],
    format: ImageFormat,
    cancelled: impl Fn() -> bool,
) -> Option<Result<PreviewContent, String>> {
    if cancelled() {
        return None;
    }
    let mut limits = Limits::default();
    // Pixel count below is stricter than these independent dimension limits.
    limits.max_image_width = Some(MAX_IMAGE_PIXELS as u32);
    limits.max_image_height = Some(MAX_IMAGE_PIXELS as u32);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoder = match reader.into_decoder() {
        Ok(decoder) => decoder,
        Err(error) => return Some(Err(format!("Could not read this image: {error}"))),
    };
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Some(Ok(PreviewContent::Unsupported(
            "This image exceeds the 16 megapixel preview limit.".into(),
        )));
    }
    if decoder.total_bytes() > MAX_IMAGE_DECODE_BYTES {
        return Some(Ok(PreviewContent::Unsupported(
            "This image exceeds the preview memory limit.".into(),
        )));
    }
    if cancelled() {
        return None;
    }
    let decoded = match DynamicImage::from_decoder(decoder) {
        Ok(decoded) => decoded,
        Err(error) => return Some(Err(format!("Could not decode this image: {error}"))),
    };
    if cancelled() {
        return None;
    }
    let resized;
    if width > MAX_PREVIEW_WIDTH || height > MAX_PREVIEW_HEIGHT {
        resized = decoded.thumbnail(MAX_PREVIEW_WIDTH, MAX_PREVIEW_HEIGHT);
        drop(decoded);
    } else {
        resized = decoded;
    }
    if cancelled() {
        return None;
    }
    let rgba = resized.into_rgba8();
    Some(Ok(PreviewContent::Image {
        size: [rgba.width() as usize, rgba.height() as usize],
        rgba: rgba.into_raw(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant, SystemTime};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let count = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let epoch = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "neptune-finder-preview-{}-{epoch}-{count}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }

        fn preview(&self, path: &Path) -> PreviewContent {
            read_preview(path, || false).unwrap().unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn text_and_empty_files_are_decoded_without_replacement() {
        let fixture = Fixture::new();
        let path = fixture.file("notes", "Neptune\nété\t世界".as_bytes());
        assert!(
            matches!(fixture.preview(&path), PreviewContent::Text(text) if text == "Neptune\nété\t世界")
        );
        let empty = fixture.file("empty", b"");
        assert!(matches!(fixture.preview(&empty), PreviewContent::Text(text) if text.is_empty()));
    }

    #[test]
    fn null_bytes_controls_and_invalid_utf8_are_not_text() {
        let fixture = Fixture::new();
        for (name, bytes) in [
            ("null", &b"hello\0world"[..]),
            ("utf8", &b"\xff\xfe"[..]),
            ("control", &b"\x01abc"[..]),
        ] {
            let path = fixture.file(name, bytes);
            assert!(matches!(
                fixture.preview(&path),
                PreviewContent::Unsupported(_)
            ));
        }
        assert!(matches!(
            fixture.preview(&fixture.0),
            PreviewContent::Unsupported(_)
        ));
    }

    #[test]
    fn text_limit_includes_notice_and_preserves_utf8_at_read_boundary() {
        let fixture = Fixture::new();
        let mut bytes = vec![b'a'; MAX_TEXT_BYTES - 1];
        bytes.extend_from_slice("🌊 more data".as_bytes());
        let path = fixture.file("long-text", &bytes);
        let PreviewContent::Text(text) = fixture.preview(&path) else {
            panic!("expected bounded text");
        };
        assert!(text.len() <= MAX_TEXT_BYTES);
        assert!(text.ends_with(TRUNCATION_NOTICE));
        assert!(!text.contains('\u{fffd}'));
    }

    #[test]
    fn exactly_limit_text_is_not_marked_truncated() {
        let fixture = Fixture::new();
        let path = fixture.file("exact-text", &vec![b'a'; MAX_TEXT_BYTES]);
        let PreviewContent::Text(text) = fixture.preview(&path) else {
            panic!("expected text");
        };
        assert_eq!(text.len(), MAX_TEXT_BYTES);
        assert!(!text.ends_with(TRUNCATION_NOTICE));
    }

    #[test]
    fn reader_caps_allocation_and_detects_an_extra_byte() {
        let mut reader = Cursor::new(vec![b'a'; 100]);
        let bounded = read_bounded(&mut reader, 7, &|| false).unwrap().unwrap();
        assert_eq!(bounded.bytes.len(), 7);
        assert!(bounded.truncated);
        assert_eq!(reader.position(), 8);
    }

    #[test]
    fn image_input_over_limit_is_rejected_without_reading_the_whole_file() {
        let fixture = Fixture::new();
        let path = fixture.file("large.png", b"\x89PNG\r\n\x1a\n");
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_IMAGE_INPUT_BYTES as u64 + 1)
            .unwrap();
        assert!(
            matches!(fixture.preview(&path), PreviewContent::Unsupported(message) if message.contains("32 MiB"))
        );
    }

    #[test]
    fn image_dimensions_are_rejected_before_pixel_decode() {
        // A valid BMP header declares oversized dimensions but supplies no pixel
        // payload. Dimension rejection must happen before truncated-data decode.
        let fixture = Fixture::new();
        let mut bmp = vec![0_u8; 54];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&54_u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&5_000_i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&5_000_i32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
        let path = fixture.file("oversized.bmp", &bmp);
        assert!(
            matches!(fixture.preview(&path), PreviewContent::Unsupported(message) if message.contains("16 megapixel"))
        );
    }

    #[test]
    fn images_are_sniffed_from_bytes_and_downsampled_with_aspect_ratio() {
        let fixture = Fixture::new();
        let path = fixture.0.join("image.without-extension");
        let image = image::RgbaImage::from_pixel(2_000, 1_000, image::Rgba([0, 120, 255, 255]));
        image.save_with_format(&path, ImageFormat::Png).unwrap();
        let PreviewContent::Image { size, rgba } = fixture.preview(&path) else {
            panic!("expected pixels");
        };
        assert_eq!(size, [1_600, 800]);
        assert_eq!(rgba.len(), size[0] * size[1] * 4);
        assert_eq!(&rgba[0..4], &[0, 120, 255, 255]);
    }

    #[test]
    fn cancellation_stops_reads_and_worker_returns_only_latest_generation() {
        let fixture = Fixture::new();
        let first = fixture.file("first", b"obsolete");
        let second = fixture.file("second", b"current");
        assert!(read_preview(&first, || true).is_none());
        let worker = Previews::new(|| {});
        worker.request(first);
        let generation = worker.request(second.clone());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = worker.poll() {
                assert_eq!(result.generation, generation);
                assert_eq!(result.path, second);
                assert!(
                    matches!(result.result.unwrap(), PreviewContent::Text(text) if text == "current")
                );
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
