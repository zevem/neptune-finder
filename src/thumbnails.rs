//! Icon-view thumbnails. Lookups, reads and decoding run on workers; a frame
//! only draws textures that are already uploaded and names the files it lacks.
//!
//! A current thumbnail in the freedesktop.org cache, saved there by GNOME
//! Files, Dolphin, Blender and others, is used first: that is how videos,
//! documents and formats this build cannot decode get pictures. Otherwise
//! images this build decodes are reduced in process. Nothing is written back.

use eframe::egui;
use finder_core::{Entry, EntryKind};
use image::{DynamicImage, ImageBuffer, ImageDecoder, ImageFormat, ImageReader, Limits};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::SystemTime;

/// Images larger than this keep their icon rather than being read whole.
const MAX_INPUT_BYTES: u64 = 64 * 1024 * 1024;
/// Decoded pixels per worker; with `MAX_WORKERS` this bounds decoding memory.
const MAX_DECODE_BYTES: u64 = 96 * 1024 * 1024;
const MAX_WORKERS: usize = 3;
/// More than a few screens of icons is never queued.
const MAX_QUEUE: usize = 512;
/// Uploaded pixels and pictures kept for files no longer on screen.
const CACHE_BYTES: usize = 64 * 1024 * 1024;
const CACHE_SLOTS: usize = 4_096;
/// The freedesktop.org cache's folders and the square each one fills.
const CACHE_SIZES: [(&str, u32); 4] = [
    ("normal", 128),
    ("large", 256),
    ("x-large", 512),
    ("xx-large", 1024),
];

/// The pixels a thumbnail needs for icons of `points`: rounded up to 32 so the
/// GPU, which has no mipmaps here, never shrinks a picture by half or more,
/// and small icon-size changes reuse the same pictures.
pub fn pixels(points: f32, pixels_per_point: f32) -> u32 {
    let exact = (points * pixels_per_point).clamp(1.0, 1024.0);
    (exact / 32.0).ceil() as u32 * 32
}

/// A file as listed, and the pixels its thumbnail fills.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
    pub pixels: u32,
}
impl Key {
    pub fn new(entry: &Entry, pixels: u32) -> Self {
        Self {
            path: entry.path.clone(),
            modified: entry.modified,
            size: entry.size,
            pixels,
        }
    }
}

pub struct Picture {
    pub image: egui::ColorImage,
    pub opaque: bool,
}

pub struct Thumbnail {
    pub key: Key,
    /// `None` when the file has no thumbnail; it keeps its icon.
    pub picture: Option<Picture>,
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<Key>,
    running: HashSet<Key>,
    done: Vec<Thumbnail>,
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    changed: Condvar,
    wake: Box<dyn Fn() + Send + Sync>,
    cache: Option<PathBuf>,
}

/// Sleeping workers that make thumbnails for the files on screen. A request
/// replaces the queue, so files scrolled away are never read; a read already
/// under way finishes within the decode limits. Drop stops the workers.
pub struct Thumbnails {
    shared: Arc<Shared>,
}

impl Thumbnails {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_cache(desktop_cache(), wake)
    }

    fn with_cache(cache: Option<PathBuf>, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            queue: Mutex::default(),
            changed: Condvar::new(),
            wake: Box::new(wake),
            cache,
        });
        // Leave a core for the window; decoding is bounded per worker.
        let workers = std::thread::available_parallelism()
            .map_or(1, |cores| cores.get().saturating_sub(1))
            .clamp(1, MAX_WORKERS);
        for _ in 0..workers {
            let worker = Arc::clone(&shared);
            // Without workers files keep their icons; nothing else waits on them.
            let _ = std::thread::Builder::new()
                .name("neptune-finder-thumbnail".into())
                .spawn(move || run(worker));
        }
        Self { shared }
    }

    /// Makes thumbnails for `keys`, in order, instead of anything still queued.
    pub fn request(&self, keys: &[Key]) {
        let mut queue = lock(&self.shared);
        let Queue {
            pending,
            running,
            done,
            ..
        } = &mut *queue;
        let mut seen = HashSet::new();
        pending.clear();
        pending.extend(
            keys.iter()
                .filter(|key| {
                    !running.contains(*key)
                        && !done.iter().any(|thumbnail| thumbnail.key == **key)
                        && seen.insert(*key)
                })
                .take(MAX_QUEUE)
                .cloned(),
        );
        drop(queue);
        self.shared.changed.notify_all();
    }

    /// Thumbnails finished since the last poll.
    pub fn poll(&self) -> Vec<Thumbnail> {
        std::mem::take(&mut lock(&self.shared).done)
    }
}

impl Drop for Thumbnails {
    fn drop(&mut self) {
        let mut queue = lock(&self.shared);
        queue.closed = true;
        queue.pending.clear();
        drop(queue);
        self.shared.changed.notify_all();
    }
}

fn lock(shared: &Shared) -> MutexGuard<'_, Queue> {
    shared
        .queue
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn run(shared: Arc<Shared>) {
    loop {
        let key = {
            let mut queue = lock(&shared);
            loop {
                if queue.closed {
                    return;
                }
                if let Some(key) = queue.pending.pop_front() {
                    queue.running.insert(key.clone());
                    break key;
                }
                queue = shared
                    .changed
                    .wait(queue)
                    .unwrap_or_else(|error| error.into_inner());
            }
        };
        let picture = render(&key, shared.cache.as_deref());
        let mut queue = lock(&shared);
        queue.running.remove(&key);
        if queue.closed {
            return;
        }
        // One wake covers every result the window has not collected yet.
        let wake = queue.done.is_empty();
        queue.done.push(Thumbnail { key, picture });
        drop(queue);
        if wake {
            (shared.wake)();
        }
    }
}

fn render(key: &Key, cache: Option<&Path>) -> Option<Picture> {
    let decodable = decodable(&key.path);
    if cache.is_none() && !decodable {
        return None;
    }
    // Never open devices, sockets or FIFOs: even reading a header can wait.
    let metadata = fs::metadata(&key.path).ok().filter(fs::Metadata::is_file)?;
    let image = cache
        .and_then(|cache| cached(cache, &key.path, &metadata, key.pixels))
        .or_else(|| {
            decodable
                .then(|| decode(&key.path, &metadata, key.pixels))
                .flatten()
        })?;
    let rgba = fit(image, key.pixels).into_rgba8();
    Some(Picture {
        image: egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        ),
        opaque: rgba.pixels().all(|pixel| pixel[3] == u8::MAX),
    })
}

/// Raster formats this build decodes, chosen by name so other files are never
/// opened for decoding; the bytes still choose the decoder.
fn decodable(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp", "bmp"]
                .iter()
                .any(|known| extension.eq_ignore_ascii_case(known))
        })
}

fn decode(path: &Path, metadata: &fs::Metadata, pixels: u32) -> Option<DynamicImage> {
    if metadata.len() > MAX_INPUT_BYTES {
        return None;
    }
    let mut reader = ImageReader::new(BufReader::new(File::open(path).ok()?))
        .with_guessed_format()
        .ok()?;
    if !matches!(
        reader.format()?,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp
    ) {
        return None;
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    // Reject by the header's dimensions before allocating pixels.
    if decoder.total_bytes() > MAX_DECODE_BYTES {
        return None;
    }
    let orientation = decoder.orientation().ok();
    let mut image = fit(DynamicImage::from_decoder(decoder).ok()?, pixels);
    // Cameras record rotation beside the pixels; show photos upright.
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    Some(image)
}

/// Shrinks to fit a square of `pixels`, never enlarging. Area averaging is
/// fast for large reductions; near the target a triangle filter avoids aliasing.
fn fit(image: DynamicImage, pixels: u32) -> DynamicImage {
    let longest = image.width().max(image.height());
    if longest <= pixels {
        image
    } else if longest >= pixels.saturating_mul(4) {
        image.thumbnail(pixels, pixels)
    } else {
        image.resize(pixels, pixels, image::imageops::FilterType::Triangle)
    }
}

/// A current thumbnail from the freedesktop.org cache: the smallest size that
/// fills `pixels`, then larger ones, then smaller ones.
fn cached(cache: &Path, path: &Path, metadata: &fs::Metadata, pixels: u32) -> Option<DynamicImage> {
    let (uri, name) = cache_name(path)?;
    let modified = metadata
        .modified()
        .ok()?
        .duration_since(SystemTime::UNIX_EPOCH)
        .ok()?
        .as_secs();
    let first = CACHE_SIZES
        .iter()
        .position(|(_, size)| *size >= pixels)
        .unwrap_or(CACHE_SIZES.len());
    CACHE_SIZES[first..]
        .iter()
        .chain(CACHE_SIZES[..first].iter().rev())
        .find_map(|(folder, _)| {
            read_cached(
                &cache.join(folder).join(&name),
                &uri,
                modified,
                metadata.len(),
            )
        })
}

/// A cached thumbnail, if it describes this version of the file: the spec
/// requires its URI and modification time to match, and its size if recorded.
fn read_cached(file: &Path, uri: &str, modified: u64, size: u64) -> Option<DynamicImage> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(file).ok()?));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    // Text may follow the image data.
    reader.finish().ok()?;
    let text = |keyword: &str| {
        reader
            .info()
            .uncompressed_latin1_text
            .iter()
            .find(|chunk| chunk.keyword == keyword)
            .map(|chunk| chunk.text.as_str())
    };
    let number = |keyword| text(keyword).and_then(|text| text.parse::<u64>().ok());
    let current = text("Thumb::URI") == Some(uri)
        && number("Thumb::MTime") == Some(modified)
        && (text("Thumb::Size").is_none() || number("Thumb::Size") == Some(size));
    if !current {
        return None;
    }
    pixels.truncate(frame.buffer_size());
    let (width, height) = (frame.width, frame.height);
    match frame.color_type {
        png::ColorType::Rgba => {
            ImageBuffer::from_raw(width, height, pixels).map(DynamicImage::ImageRgba8)
        }
        png::ColorType::Rgb => {
            ImageBuffer::from_raw(width, height, pixels).map(DynamicImage::ImageRgb8)
        }
        png::ColorType::GrayscaleAlpha => {
            ImageBuffer::from_raw(width, height, pixels).map(DynamicImage::ImageLumaA8)
        }
        png::ColorType::Grayscale => {
            ImageBuffer::from_raw(width, height, pixels).map(DynamicImage::ImageLuma8)
        }
        png::ColorType::Indexed => None,
    }
}

/// `$XDG_CACHE_HOME/thumbnails`, shared by freedesktop.org desktops.
#[cfg(all(unix, not(target_os = "macos")))]
fn desktop_cache() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.cache_dir().join("thumbnails"))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn desktop_cache() -> Option<PathBuf> {
    None
}

/// A path's `file://` URI and its cache file name, the URI's MD5. The URI is
/// escaped as GLib escapes it, so names match other applications' thumbnails.
#[cfg(all(unix, not(target_os = "macos")))]
fn cache_name(path: &Path) -> Option<(String, String)> {
    use md5::{Digest, Md5};
    use std::fmt::Write;
    use std::os::unix::ffi::OsStrExt;
    if !path.is_absolute() {
        return None;
    }
    let mut uri = String::from("file://");
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"!$&'()*+,-./:=@_~".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            let _ = write!(uri, "%{byte:02X}");
        }
    }
    let name = format!("{:x}.png", Md5::digest(uri.as_bytes()));
    Some((uri, name))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn cache_name(_: &Path) -> Option<(String, String)> {
    None
}

/// An uploaded thumbnail.
pub struct Texture {
    pub handle: egui::TextureHandle,
    /// Opaque pictures get an edge; transparent ones keep their own outline.
    pub opaque: bool,
}

struct Slot {
    key: Key,
    texture: Option<Texture>,
    bytes: usize,
    used: u64,
}

/// The window's uploaded thumbnails. Frames look up the files they draw and
/// collect the ones still missing. Beyond a memory budget, the pictures drawn
/// least recently are forgotten; pictures on screen always stay.
#[derive(Default)]
pub struct Cache {
    slots: HashMap<PathBuf, Slot>,
    wanted: Vec<Key>,
    bytes: usize,
    frame: u64,
}

impl Cache {
    /// The picture to draw for `entry`, if it has one. A changed file or icon
    /// size asks for a new picture; the previous one shows until it arrives.
    pub fn get(&mut self, entry: &Entry, pixels: u32) -> Option<&Texture> {
        if !matches!(entry.kind, EntryKind::File | EntryKind::Symlink) {
            return None;
        }
        match self.slots.get_mut(&entry.path) {
            Some(slot) => {
                slot.used = self.frame;
                let key = &slot.key;
                if key.modified != entry.modified || key.size != entry.size || key.pixels != pixels
                {
                    self.wanted.push(Key::new(entry, pixels));
                }
                slot.texture.as_ref()
            }
            None => {
                self.wanted.push(Key::new(entry, pixels));
                None
            }
        }
    }

    /// The picture already uploaded for `entry`, without asking for one.
    pub fn peek(&self, entry: &Entry) -> Option<&Texture> {
        self.slots.get(&entry.path)?.texture.as_ref()
    }

    pub fn insert(&mut self, ctx: &egui::Context, thumbnail: Thumbnail) {
        let bytes = thumbnail
            .picture
            .as_ref()
            .map_or(0, |picture| picture.image.pixels.len() * 4);
        let texture = thumbnail.picture.map(|picture| Texture {
            // Texture names reach debugging tools, so never a file name.
            handle: ctx.load_texture("thumbnail", picture.image, egui::TextureOptions::LINEAR),
            opaque: picture.opaque,
        });
        let slot = Slot {
            key: thumbnail.key,
            texture,
            bytes,
            used: self.frame,
        };
        self.bytes += bytes;
        if let Some(old) = self.slots.insert(slot.key.path.clone(), slot) {
            self.bytes -= old.bytes;
        }
    }

    /// Ends a frame: returns the thumbnails it still needs, in drawing order,
    /// and trims pictures not drawn in it to three quarters of the budget.
    pub fn end_frame(&mut self) -> Vec<Key> {
        if self.bytes > CACHE_BYTES || self.slots.len() > CACHE_SLOTS {
            let mut idle = self
                .slots
                .iter()
                .filter(|(_, slot)| slot.used != self.frame)
                .map(|(path, slot)| (slot.used, path.clone()))
                .collect::<Vec<_>>();
            idle.sort_unstable();
            for (_, path) in idle {
                if self.bytes <= CACHE_BYTES / 4 * 3 && self.slots.len() <= CACHE_SLOTS / 4 * 3 {
                    break;
                }
                if let Some(slot) = self.slots.remove(&path) {
                    self.bytes -= slot.bytes;
                }
            }
        }
        self.frame += 1;
        std::mem::take(&mut self.wanted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

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
                "neptune-finder-thumbnails-{}-{epoch}-{count}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn image(&self, name: &str, width: u32, height: u32, alpha: u8) -> PathBuf {
            let path = self.0.join(name);
            image::RgbaImage::from_pixel(width, height, image::Rgba([0, 120, 255, alpha]))
                .save_with_format(&path, ImageFormat::Png)
                .unwrap();
            path
        }

        fn key(&self, path: &Path, pixels: u32) -> Key {
            Key {
                path: path.to_owned(),
                modified: None,
                size: 0,
                pixels,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn size(picture: &Picture) -> [usize; 2] {
        picture.image.size
    }

    fn entry(path: &Path, kind: EntryKind) -> Entry {
        Entry {
            path: path.to_owned(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            kind,
            size: 1,
            modified: Some(SystemTime::UNIX_EPOCH),
            hidden: false,
        }
    }

    fn picture(width: usize, height: usize) -> Option<Picture> {
        Some(Picture {
            image: egui::ColorImage::filled([width, height], egui::Color32::WHITE),
            opaque: true,
        })
    }

    #[test]
    fn pixels_round_up_so_the_gpu_never_halves_a_picture() {
        assert_eq!(pixels(32.0, 1.0), 32);
        assert_eq!(pixels(33.0, 1.0), 64);
        assert_eq!(pixels(56.0, 1.0), 64);
        assert_eq!(pixels(56.0, 2.0), 128);
        assert_eq!(pixels(96.0, 2.0), 192);
        assert_eq!(pixels(f32::MAX, 3.0), 1024);
    }

    #[test]
    fn images_shrink_to_fit_with_aspect_ratio_and_are_never_enlarged() {
        let fixture = Fixture::new();
        let wide = fixture.image("wide.png", 2_000, 1_000, 255);
        let picture = render(&fixture.key(&wide, 128), None).expect("thumbnail");
        assert_eq!(size(&picture), [128, 64]);
        assert!(picture.opaque);
        let near = fixture.image("near.png", 300, 150, 255);
        assert_eq!(
            size(&render(&fixture.key(&near, 192), None).unwrap()),
            [192, 96]
        );
        let small = fixture.image("small.png", 20, 10, 128);
        let picture = render(&fixture.key(&small, 128), None).expect("thumbnail");
        assert_eq!(size(&picture), [20, 10]);
        assert!(!picture.opaque);
    }

    #[test]
    fn other_files_folders_and_broken_images_keep_their_icons() {
        let fixture = Fixture::new();
        let text = fixture.0.join("notes.txt");
        fs::write(&text, "Neptune").unwrap();
        let broken = fixture.0.join("broken.png");
        fs::write(&broken, b"\x89PNG\r\n\x1a\nnot really").unwrap();
        let folder = fixture.0.join("folder.png");
        fs::create_dir(&folder).unwrap();
        // Decoding is chosen by name; a PNG without one is not opened.
        let unnamed = fixture.image("picture.bin", 8, 8, 255);
        let missing = fixture.0.join("missing.png");
        for path in [text, broken, folder, unnamed, missing] {
            assert!(render(&fixture.key(&path, 64), None).is_none());
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn cache_names_match_glib_escaping_and_md5() {
        let (uri, name) = cache_name(Path::new(
            "/home/neptune/Pictures/Screenshot From 2026-10-18 [1] été.png",
        ))
        .unwrap();
        assert_eq!(
            uri,
            "file:///home/neptune/Pictures/Screenshot%20From%202026-10-18%20%5B1%5D%20%C3%A9t%C3%A9.png"
        );
        assert_eq!(name, "00db9b05c81e5d891eaf5dadf76a99bc.png");
        assert_eq!(
            cache_name(Path::new("/a/b!$&'()*+,-.:=@_~#?%;")).unwrap().0,
            "file:///a/b!$&'()*+,-.:=@_~%23%3F%25%3B"
        );
        assert!(cache_name(Path::new("relative.png")).is_none());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn cached_thumbnails_are_used_only_while_they_describe_the_file() {
        let fixture = Fixture::new();
        let cache = fixture.0.join("thumbnails");
        // Not decodable here: only the desktop's cache can picture it.
        let video = fixture.0.join("clip.mp4");
        fs::write(&video, b"not a real video").unwrap();
        let metadata = fs::metadata(&video).unwrap();
        let modified = metadata
            .modified()
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let (uri, name) = cache_name(&video).unwrap();
        let save = |folder: &str, text: &[(&str, String)]| {
            fs::create_dir_all(cache.join(folder)).unwrap();
            let file = File::create(cache.join(folder).join(&name)).unwrap();
            let mut encoder = png::Encoder::new(file, 64, 32);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            for (keyword, value) in text {
                encoder
                    .add_text_chunk((*keyword).into(), value.clone())
                    .unwrap();
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[200; 64 * 32 * 3]).unwrap();
        };
        let key = fixture.key(&video, 128);
        assert!(render(&key, Some(&cache)).is_none());

        save(
            "x-large",
            &[
                ("Thumb::URI", uri.clone()),
                ("Thumb::MTime", modified.to_string()),
            ],
        );
        let picture = render(&key, Some(&cache)).expect("cached thumbnail");
        assert_eq!(size(&picture), [64, 32]);
        assert!(picture.opaque);

        // A stale thumbnail in the preferred size is passed over.
        save(
            "normal",
            &[
                ("Thumb::URI", uri.clone()),
                ("Thumb::MTime", (modified + 1).to_string()),
            ],
        );
        assert!(render(&key, Some(&cache)).is_some());
        for text in [
            vec![("Thumb::URI", uri.clone())],
            vec![
                ("Thumb::URI", "file:///elsewhere/clip.mp4".into()),
                ("Thumb::MTime", modified.to_string()),
            ],
            vec![
                ("Thumb::URI", uri.clone()),
                ("Thumb::MTime", modified.to_string()),
                ("Thumb::Size", (metadata.len() + 1).to_string()),
            ],
        ] {
            save("x-large", &text);
            assert!(render(&key, Some(&cache)).is_none());
        }
    }

    #[test]
    fn workers_answer_each_requested_file_once() {
        let fixture = Fixture::new();
        let image = fixture.image("photo.png", 400, 200, 255);
        let text = fixture.0.join("notes.txt");
        fs::write(&text, "Neptune").unwrap();
        let workers = Thumbnails::with_cache(None, || {});
        let keys = [
            fixture.key(&image, 64),
            fixture.key(&image, 64),
            fixture.key(&text, 64),
        ];
        workers.request(&keys);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut results = Vec::new();
        while results.len() < 2 {
            assert!(Instant::now() < deadline, "thumbnails did not arrive");
            results.extend(workers.poll());
            std::thread::sleep(Duration::from_millis(2));
        }
        std::thread::sleep(Duration::from_millis(20));
        results.extend(workers.poll());
        assert_eq!(results.len(), 2);
        let photo = results
            .iter()
            .find(|result| result.key.path == image)
            .unwrap();
        assert_eq!(size(photo.picture.as_ref().unwrap()), [64, 32]);
        let notes = results
            .iter()
            .find(|result| result.key.path == text)
            .unwrap();
        assert!(notes.picture.is_none());
    }

    #[test]
    fn cache_asks_for_missing_and_changed_files_and_shows_the_last_picture() {
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        let photo = entry(Path::new("/pictures/photo.png"), EntryKind::File);
        let folder = entry(Path::new("/pictures/album"), EntryKind::Directory);
        assert!(cache.get(&photo, 64).is_none());
        assert!(cache.get(&folder, 64).is_none());
        assert_eq!(cache.end_frame(), vec![Key::new(&photo, 64)]);

        cache.insert(
            &ctx,
            Thumbnail {
                key: Key::new(&photo, 64),
                picture: picture(4, 2),
            },
        );
        assert!(cache.get(&photo, 64).is_some());
        assert!(cache.end_frame().is_empty());

        let edited = Entry {
            size: 2,
            ..photo.clone()
        };
        assert!(cache.get(&edited, 64).is_some());
        assert!(cache.get(&photo, 128).is_some());
        assert_eq!(
            cache.end_frame(),
            vec![Key::new(&edited, 64), Key::new(&photo, 128)]
        );
    }

    #[test]
    fn cache_forgets_pictures_drawn_least_recently_and_keeps_those_on_screen() {
        let ctx = egui::Context::default();
        let mut cache = Cache::default();
        let add = |cache: &mut Cache, batch: &str, count: usize| {
            let paths = (0..count)
                .map(|index| PathBuf::from(format!("/{batch}/{index}.png")))
                .collect::<Vec<_>>();
            for path in &paths {
                cache.insert(
                    &ctx,
                    Thumbnail {
                        key: Key::new(&entry(path, EntryKind::File), 64),
                        picture: None,
                    },
                );
            }
            paths
        };
        let older = add(&mut cache, "older", 3_500);
        assert!(cache.end_frame().is_empty());
        assert_eq!(cache.slots.len(), 3_500);
        // The next frame exceeds the budget while the oldest picture is drawn.
        let newer = add(&mut cache, "newer", 600);
        assert!(cache.get(&entry(&older[0], EntryKind::File), 64).is_none());
        cache.end_frame();
        assert!(cache.slots.len() <= CACHE_SLOTS / 4 * 3);
        assert!(cache.slots.contains_key(&older[0]));
        assert!(newer.iter().all(|path| cache.slots.contains_key(path)));
    }
}
