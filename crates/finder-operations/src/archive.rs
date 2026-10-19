//! ZIP compression and extraction, staged privately beside the destination.
//!
//! An archive is written into private staging and published with a native
//! no-replace rename. Extraction never writes outside its staging folder:
//! entry names must stay inside it, and links it recreates are never
//! traversed. Its result is published under a free name, so an existing item
//! is never replaced or merged.

use super::{
    MAX_COPY_DEPTH, checked_directory, checked_item, checked_selection, describe, history::Journal,
    item_word, new_child, platform, unchanged_metadata, validate_name,
};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, FileTimes, Metadata, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ExtraField, ZipArchive, ZipWriter};

/// Most items one archive may contain or be built from.
const MAX_ARCHIVE_ENTRIES: usize = 250_000;
/// DEFLATE cannot expand data more than about 1,032 times. An archive that
/// declares more is malformed or built to exhaust the disk.
const MAX_EXPANSION: u64 = 1_100;
/// Longest symbolic link target read from an archive.
const MAX_LINK_TARGET: u64 = 4_096;
/// Numbered names tried for an extracted item before giving up.
const MAX_NAME_ATTEMPTS: u32 = 1_000;
/// Resource forks that macOS adds beside the real items of a ZIP archive.
const MAC_RESOURCES: &str = "__MACOSX";

/// Whether Neptune Finder extracts this file itself, judged by name alone.
pub fn is_extractable(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

pub(super) fn compress(
    sources: Vec<PathBuf>,
    destination: &Path,
    name: &str,
    journal: &mut Journal,
) -> Result<String, String> {
    let target = new_child(destination, name)?;
    let destination = target
        .parent()
        .ok_or("The archive destination has no parent folder")?;
    let sources = checked_selection(sources)?;
    let mut names = HashSet::with_capacity(sources.len());
    let mut items = Vec::with_capacity(sources.len());
    for source in sources {
        if destination.starts_with(&source) {
            return Err(format!(
                "A folder cannot be compressed into itself: {}",
                source.display()
            ));
        }
        let item = entry_name(&source)?;
        if !names.insert(item.clone()) {
            return Err(format!(
                "Multiple selected items are named {item}; an archive needs unique names"
            ));
        }
        items.push((source, item));
    }
    let stage = tempfile::Builder::new()
        .prefix(".neptune-compress-")
        .tempdir_in(destination)
        .map_err(|error| describe("prepare archive in", destination, error))?;
    let payload = stage.path().join("archive.zip");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&payload)
        .map_err(|error| describe("create archive", &target, error))?;
    let mut writer = ZipWriter::new(BufWriter::new(file));
    let mut count = 0;
    for (source, item) in &items {
        add(&mut writer, source, item, 0, &mut count)?;
    }
    let file = writer
        .finish()
        .map_err(|error| format!("Could not finish the archive: {error}"))?
        .into_inner()
        .map_err(|error| describe("write archive", &target, error.into_error()))?;
    file.sync_all()
        .map_err(|error| describe("finish archive", &target, error))?;
    drop(file);
    platform::rename_noreplace(&payload, &target)
        .map_err(|error| describe("publish archive", &target, error))?;
    journal.created(&target);
    Ok(match items.as_slice() {
        [(_, item)] => format!("Compressed {item} into {name}"),
        _ => format!(
            "Compressed {} {} into {name}",
            items.len(),
            item_word(items.len())
        ),
    })
}

/// An item's name inside an archive, which ZIP stores as Unicode text.
fn entry_name(path: &Path) -> Result<String, String> {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            format!(
                "A ZIP archive can only store Unicode names: {}",
                path.display()
            )
        })
}

fn add(
    writer: &mut ZipWriter<BufWriter<File>>,
    path: &Path,
    name: &str,
    depth: usize,
    count: &mut usize,
) -> Result<(), String> {
    if depth > MAX_COPY_DEPTH {
        return Err(format!(
            "The folder tree is deeper than the supported {MAX_COPY_DEPTH} levels: {}",
            path.display()
        ));
    }
    *count += 1;
    if *count > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "An archive can contain at most {MAX_ARCHIVE_ENTRIES} items"
        ));
    }
    let failed =
        |error: ZipError| format!("Could not add {} to the archive: {error}", path.display());
    let metadata = fs::symlink_metadata(path).map_err(|error| describe("inspect", path, error))?;
    let options = entry_options(&metadata);
    if metadata.file_type().is_symlink() {
        let target =
            fs::read_link(path).map_err(|error| describe("read symbolic link", path, error))?;
        let target = target.to_str().ok_or_else(|| {
            format!(
                "A ZIP archive can only store Unicode link targets: {}",
                path.display()
            )
        })?;
        return writer.add_symlink(name, target, options).map_err(failed);
    }
    if metadata.is_dir() {
        writer.add_directory(name, options).map_err(failed)?;
        let mut children = Vec::new();
        for entry in fs::read_dir(path).map_err(|error| describe("read folder", path, error))? {
            let entry = entry.map_err(|error| describe("read folder entry", path, error))?;
            children.push(entry.path());
        }
        // A stable order makes archives of the same tree identical.
        children.sort();
        for child in children {
            let child_name = entry_name(&child)?;
            add(
                writer,
                &child,
                &format!("{name}/{child_name}"),
                depth + 1,
                count,
            )?;
        }
        let after =
            fs::symlink_metadata(path).map_err(|error| describe("verify folder", path, error))?;
        return unchanged(path, &metadata, &after);
    }
    if !metadata.is_file() {
        return Err(format!(
            "Special files (devices, sockets, and pipes) cannot be compressed: {}",
            path.display()
        ));
    }
    let mut input =
        platform::open_regular_file(path).map_err(|error| describe("open", path, error))?;
    let opened = input
        .metadata()
        .map_err(|error| describe("inspect opened file", path, error))?;
    unchanged(path, &metadata, &opened)?;
    writer
        .start_file(
            name,
            options.large_file(opened.len() >= zip::ZIP64_BYTES_THR),
        )
        .map_err(failed)?;
    let copied = io::copy(&mut input, writer).map_err(|error| describe("compress", path, error))?;
    let after = input
        .metadata()
        .map_err(|error| describe("verify compressed file", path, error))?;
    unchanged(path, &opened, &after)?;
    if copied != opened.len() {
        return Err(format!(
            "{} changed length while it was compressed. Retry when writes have finished",
            path.display()
        ));
    }
    Ok(())
}

fn unchanged(path: &Path, before: &Metadata, after: &Metadata) -> Result<(), String> {
    if unchanged_metadata(before, after) {
        Ok(())
    } else {
        Err(format!(
            "{} changed while it was compressed. Retry when writes have finished; the incomplete archive was discarded",
            path.display()
        ))
    }
}

fn entry_options(metadata: &Metadata) -> SimpleFileOptions {
    // ZIP stores local wall-clock time, as other archivers read it.
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| {
            zip::DateTime::try_from(chrono::DateTime::<chrono::Local>::from(time).naive_local())
                .ok()
        })
        .unwrap_or_default();
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(modified);
    #[cfg(unix)]
    let options = {
        use std::os::unix::fs::PermissionsExt;
        options.unix_permissions(metadata.permissions().mode() & 0o777)
    };
    options
}

pub(super) fn extract(
    archive: &Path,
    destination: &Path,
    journal: &mut Journal,
) -> Result<String, String> {
    if !is_extractable(archive) {
        return Err(format!(
            "Only ZIP archives can be extracted: {}",
            archive.display()
        ));
    }
    let archive = checked_item(archive)?;
    let destination = checked_directory(destination)?;
    let file = File::open(&archive).map_err(|error| describe("open archive", &archive, error))?;
    let length = file
        .metadata()
        .map_err(|error| describe("inspect archive", &archive, error))?
        .len();
    let unreadable = |error: ZipError| match error {
        ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => format!(
            "{} is protected by a password, which Neptune Finder can't open yet",
            archive.display()
        ),
        error => format!("Could not read {}: {error}", archive.display()),
    };
    let mut zip = ZipArchive::new(BufReader::new(file)).map_err(unreadable)?;
    if zip.len() > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "{} contains more than {MAX_ARCHIVE_ENTRIES} items",
            archive.display()
        ));
    }
    // Shared data lets a small archive declare unlimited output.
    if zip.has_overlapping_files().map_err(unreadable)? {
        return Err(format!(
            "{} reuses compressed data for several items, as archive bombs do. Nothing was extracted",
            archive.display()
        ));
    }
    let mut declared = 0u64;
    for index in 0..zip.len() {
        declared = declared.saturating_add(zip.by_index_data(index).map_err(unreadable)?.size());
    }
    if declared > length.saturating_mul(MAX_EXPANSION) {
        return Err(format!(
            "{} claims to expand more than ZIP compression allows. Nothing was extracted",
            archive.display()
        ));
    }

    let stage = tempfile::Builder::new()
        .prefix(".neptune-extract-")
        .tempdir_in(&destination)
        .map_err(|error| describe("prepare extraction in", &destination, error))?;
    let root = stage.path().join("item");
    fs::create_dir(&root)
        .map_err(|error| describe("prepare extraction in", &destination, error))?;
    let staging = Staging { root: root.clone() };
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(unreadable)?;
        let name = entry
            .name()
            .map(|name| name.into_owned())
            .unwrap_or_default();
        let relative = entry_path(&name, entry.enclosed_name())?;
        if relative.as_os_str().is_empty() || relative.starts_with(MAC_RESOURCES) {
            continue;
        }
        let mode = entry.unix_mode();
        if entry.is_dir() {
            staging.folder(&relative, mode)?;
        } else if entry.is_symlink() {
            let mut target = String::new();
            (&mut entry)
                .take(MAX_LINK_TARGET + 1)
                .read_to_string(&mut target)
                .map_err(|error| entry_error("read", &relative, error))?;
            staging.link(&relative, &target)?;
        } else {
            let size = entry.size();
            let modified = modified(&entry);
            staging.file(&relative, &mut entry, size, mode, modified)?;
        }
    }

    // One top-level item is published as itself; several share a folder
    // named after the archive, as the system archive utilities do.
    let mut children = fs::read_dir(&root)
        .map_err(|error| describe("read extracted items in", &destination, error))?
        .take(2)
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<io::Result<Vec<_>>>()
        .map_err(|error| describe("read extracted items in", &destination, error))?;
    let (source, name) = match children.len() {
        0 => {
            return Err(format!("{} has no items to extract", archive.display()));
        }
        1 => {
            let name = children.remove(0);
            (root.join(&name), name)
        }
        _ => (root, archive_stem(&archive)),
    };
    let folder = fs::symlink_metadata(&source).is_ok_and(|metadata| metadata.is_dir());
    let target = publish(&source, &destination, &name, folder)?;
    journal.created(&target);
    Ok(format!(
        "Extracted {}",
        target
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default()
    ))
}

/// The entry's location inside the extracted folder. Names that leave the
/// folder, or that this system cannot store, fail the whole extraction.
fn entry_path(name: &str, enclosed: Option<PathBuf>) -> Result<PathBuf, String> {
    let unsafe_name = || {
        format!(
            "The archive contains an item that can't be extracted safely: {name}. Nothing was extracted"
        )
    };
    let path = enclosed.ok_or_else(unsafe_name)?;
    let mut depth = 0;
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(unsafe_name());
        };
        validate_name(part.to_str().ok_or_else(unsafe_name)?).map_err(|_| unsafe_name())?;
        depth += 1;
    }
    if depth > MAX_COPY_DEPTH {
        return Err(format!(
            "The archive is nested deeper than the supported {MAX_COPY_DEPTH} levels"
        ));
    }
    Ok(path)
}

/// The archive's own record of when a file changed: its exact UTC time when
/// present, otherwise its local wall-clock time.
fn modified<R: Read>(entry: &zip::read::ZipFile<'_, R>) -> Option<SystemTime> {
    let exact = entry.extra_data_fields().find_map(|field| match field {
        ExtraField::ExtendedTimestamp(timestamp) => timestamp.mod_time(),
        _ => None,
    });
    if let Some(seconds) = exact {
        return SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds.into()));
    }
    let local = chrono::NaiveDateTime::try_from(entry.last_modified()?).ok()?;
    local
        .and_local_timezone(chrono::Local)
        .earliest()
        .map(SystemTime::from)
}

/// The extraction folder in private staging. Every write creates a new item
/// below real folders, so nothing is written through a link or over an item.
struct Staging {
    root: PathBuf,
}

impl Staging {
    /// The staged path for `relative`, creating its missing parent folders.
    fn prepare(&self, relative: &Path) -> Result<PathBuf, String> {
        let mut path = self.root.clone();
        let mut components = relative.components().peekable();
        while let Some(component) = components.next() {
            path.push(component);
            if components.peek().is_none() {
                break;
            }
            match fs::symlink_metadata(&path) {
                Ok(metadata) if metadata.is_dir() => {}
                Ok(_) => {
                    return Err(format!(
                        "The archive places items inside a link or file: {}. Nothing was extracted",
                        relative.display()
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&path)
                        .map_err(|error| entry_error("create", relative, error))?;
                }
                Err(error) => return Err(entry_error("inspect", relative, error)),
            }
        }
        Ok(path)
    }

    fn folder(&self, relative: &Path, mode: Option<u32>) -> Result<(), String> {
        let path = self.prepare(relative)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => Ok(()),
            Ok(_) => Err(duplicate(relative)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                #[cfg(unix)]
                if let Some(mode) = mode {
                    use std::os::unix::fs::DirBuilderExt;
                    // The process umask applies. The folder stays yours to
                    // open, change and publish.
                    builder.mode(mode & 0o777 | 0o700);
                }
                #[cfg(not(unix))]
                let _ = mode;
                builder
                    .create(&path)
                    .map_err(|error| entry_error("create", relative, error))
            }
            Err(error) => Err(entry_error("inspect", relative, error)),
        }
    }

    fn link(&self, relative: &Path, target: &str) -> Result<(), String> {
        if target.is_empty() || target.contains('\0') || target.len() as u64 > MAX_LINK_TARGET {
            return Err(format!(
                "The archive contains a damaged link: {}",
                relative.display()
            ));
        }
        let path = self.prepare(relative)?;
        platform::create_symlink(Path::new(target), &path).map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => duplicate(relative),
            _ => entry_error("create link", relative, error),
        })
    }

    fn file(
        &self,
        relative: &Path,
        contents: &mut impl Read,
        size: u64,
        mode: Option<u32>,
        modified: Option<SystemTime>,
    ) -> Result<(), String> {
        let path = self.prepare(relative)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::OpenOptionsExt;
            // The process umask applies; set-id bits are never restored.
            options.mode(mode & 0o777);
        }
        #[cfg(not(unix))]
        let _ = mode;
        let mut output = options.open(&path).map_err(|error| match error.kind() {
            io::ErrorKind::AlreadyExists => duplicate(relative),
            _ => entry_error("create", relative, error),
        })?;
        let written = io::copy(&mut contents.take(size.saturating_add(1)), &mut output)
            .map_err(|error| entry_error("extract", relative, error))?;
        if written != size {
            return Err(format!(
                "{} is damaged in the archive. Nothing was extracted",
                relative.display()
            ));
        }
        if let Some(modified) = modified {
            // An archive's time may predate what this filesystem can store;
            // the contents matter more than the timestamp.
            let _ = output.set_times(FileTimes::new().set_modified(modified));
        }
        Ok(())
    }
}

fn duplicate(relative: &Path) -> String {
    format!(
        "The archive contains {} more than once. Nothing was extracted",
        relative.display()
    )
}

fn entry_error(action: &str, relative: &Path, error: io::Error) -> String {
    format!(
        "Could not {action} {} from the archive: {error}",
        relative.display()
    )
}

/// The archive's name without its extension, for the folder that holds
/// several extracted items.
fn archive_stem(archive: &Path) -> OsString {
    archive
        .file_stem()
        .filter(|stem| !stem.is_empty())
        .unwrap_or(OsStr::new("Archive"))
        .to_owned()
}

/// Move an extracted item into place under the first free name: “Name”,
/// then “Name 2”, “Name 3”. The native rename never replaces an item.
fn publish(
    source: &Path,
    destination: &Path,
    name: &OsStr,
    folder: bool,
) -> Result<PathBuf, String> {
    for attempt in 1..=MAX_NAME_ATTEMPTS {
        let target = destination.join(numbered(name, attempt, folder));
        match platform::rename_noreplace(source, &target) {
            Ok(()) => return Ok(target),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(describe("publish extracted item", &target, error)),
        }
    }
    Err(format!(
        "{} and {MAX_NAME_ATTEMPTS} numbered alternatives already exist; existing items are never overwritten",
        destination.join(name).display()
    ))
}

fn numbered(name: &OsStr, attempt: u32, folder: bool) -> OsString {
    if attempt == 1 {
        return name.to_owned();
    }
    let path = Path::new(name);
    let mut numbered = OsString::new();
    match (folder, path.file_stem(), path.extension()) {
        (false, Some(stem), Some(extension)) => {
            numbered.push(stem);
            numbered.push(format!(" {attempt}."));
            numbered.push(extension);
        }
        _ => {
            numbered.push(name);
            numbered.push(format!(" {attempt}"));
        }
    }
    numbered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Operation, execute};
    use std::io::Write;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("create isolated test fixture");
        let path = fs::canonicalize(directory.path()).expect("canonical test fixture");
        (directory, path)
    }

    fn zip_file(path: &Path, build: impl FnOnce(&mut ZipWriter<File>)) {
        let mut writer = ZipWriter::new(File::create(path).unwrap());
        build(&mut writer);
        writer.finish().unwrap();
    }

    fn write_entry(writer: &mut ZipWriter<File>, name: &str, contents: &[u8]) {
        writer
            .start_file(name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
    }

    fn names(folder: &Path) -> Vec<String> {
        let mut names = fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    fn extract_into(archive: &Path, destination: &Path) -> Result<String, String> {
        execute(Operation::Extract {
            archive: archive.to_owned(),
            destination: destination.to_owned(),
        })
    }

    #[test]
    fn only_zip_names_are_extracted_in_place() {
        for name in ["a.zip", "B.ZIP", "notes.v2.Zip"] {
            assert!(is_extractable(Path::new(name)), "{name}");
        }
        for name in ["zip", "a.zip.part", "a.tar.gz", "a.docx", "a.jar", ".zip"] {
            assert!(!is_extractable(Path::new(name)), "{name}");
        }
    }

    #[test]
    fn compressed_tree_extracts_back_to_its_own_folder() {
        let (_fixture, root) = fixture();
        fs::create_dir_all(root.join("Project/nested")).unwrap();
        fs::write(root.join("Project/nested/notes.txt"), b"hello Neptune").unwrap();
        fs::write(root.join("Project/empty"), b"").unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        File::options()
            .write(true)
            .open(root.join("Project/nested/notes.txt"))
            .unwrap()
            .set_modified(modified)
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                root.join("Project/empty"),
                fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            std::os::unix::fs::symlink("nested/notes.txt", root.join("Project/link")).unwrap();
        }
        let message = execute(Operation::Compress {
            sources: vec![root.join("Project")],
            destination: root.clone(),
            name: "Project.zip".into(),
        })
        .unwrap();
        assert_eq!(message, "Compressed Project into Project.zip");
        assert_eq!(names(&root), ["Project", "Project.zip"]);

        fs::create_dir(root.join("out")).unwrap();
        let message = extract_into(&root.join("Project.zip"), &root.join("out")).unwrap();
        assert_eq!(message, "Extracted Project");
        assert_eq!(names(&root.join("out")), ["Project"]);
        let extracted = root.join("out/Project");
        assert_eq!(
            fs::read(extracted.join("nested/notes.txt")).unwrap(),
            b"hello Neptune"
        );
        assert_eq!(fs::read(extracted.join("empty")).unwrap(), b"");
        let restored = fs::metadata(extracted.join("nested/notes.txt"))
            .unwrap()
            .modified()
            .unwrap();
        // ZIP keeps local wall-clock time to two seconds.
        let difference = restored
            .duration_since(modified)
            .unwrap_or_else(|error| error.duration());
        assert!(difference <= Duration::from_secs(2), "{difference:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(extracted.join("empty"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o700, 0o700);
            assert_eq!(
                fs::read_link(extracted.join("link")).unwrap(),
                Path::new("nested/notes.txt")
            );
        }
        // Original stays, and no staging is left behind.
        assert_eq!(
            fs::read(root.join("Project/nested/notes.txt")).unwrap(),
            b"hello Neptune"
        );
        assert_eq!(names(&root), ["Project", "Project.zip", "out"]);
    }

    #[test]
    fn several_items_share_a_folder_and_never_replace_existing_ones() {
        let (_fixture, root) = fixture();
        fs::write(root.join("a.txt"), b"a").unwrap();
        fs::write(root.join("b.txt"), b"b").unwrap();
        execute(Operation::Compress {
            sources: vec![root.join("a.txt"), root.join("b.txt")],
            destination: root.clone(),
            name: "Pair.zip".into(),
        })
        .unwrap();
        fs::write(root.join("Pair"), b"an unrelated file").unwrap();
        assert_eq!(
            extract_into(&root.join("Pair.zip"), &root).unwrap(),
            "Extracted Pair 2"
        );
        assert_eq!(
            extract_into(&root.join("Pair.zip"), &root).unwrap(),
            "Extracted Pair 3"
        );
        assert_eq!(fs::read(root.join("Pair")).unwrap(), b"an unrelated file");
        assert_eq!(names(&root.join("Pair 2")), ["a.txt", "b.txt"]);
        assert_eq!(fs::read(root.join("Pair 3/b.txt")).unwrap(), b"b");

        zip_file(&root.join("single.zip"), |writer| {
            write_entry(writer, "a.txt", b"from the archive");
        });
        assert_eq!(
            extract_into(&root.join("single.zip"), &root).unwrap(),
            "Extracted a 2.txt"
        );
        assert_eq!(fs::read(root.join("a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(root.join("a 2.txt")).unwrap(), b"from the archive");
    }

    #[test]
    fn escaping_names_and_writes_through_links_extract_nothing() {
        let (_fixture, root) = fixture();
        fs::create_dir_all(root.join("destination")).unwrap();
        fs::create_dir_all(root.join("outside")).unwrap();
        zip_file(&root.join("escape.zip"), |writer| {
            write_entry(writer, "inside.txt", b"fine");
            write_entry(writer, "../escaped.txt", b"outside");
        });
        let error = extract_into(&root.join("escape.zip"), &root.join("destination")).unwrap_err();
        assert!(error.contains("extracted safely"), "{error}");
        zip_file(&root.join("through-link.zip"), |writer| {
            writer
                .add_symlink(
                    "link",
                    root.join("outside").to_str().unwrap(),
                    SimpleFileOptions::default(),
                )
                .unwrap();
            write_entry(writer, "link/planted.txt", b"outside");
        });
        let error =
            extract_into(&root.join("through-link.zip"), &root.join("destination")).unwrap_err();
        assert!(error.contains("inside a link"), "{error}");
        assert!(names(&root.join("outside")).is_empty());
        assert!(!root.join("escaped.txt").exists());
        // A failed extraction leaves no partial result or staging folder.
        assert!(names(&root.join("destination")).is_empty());
    }

    #[test]
    fn overlapping_data_is_refused_as_an_archive_bomb() {
        let (_fixture, root) = fixture();
        zip_file(&root.join("bomb.zip"), |writer| {
            write_entry(writer, "one", &[0; 4_096]);
            writer.shallow_copy_file("one", "two").unwrap();
        });
        let error = extract_into(&root.join("bomb.zip"), &root).unwrap_err();
        assert!(error.contains("archive bombs"), "{error}");
        assert_eq!(names(&root), ["bomb.zip"]);
    }

    #[test]
    fn impossible_declared_expansion_is_refused_before_writing() {
        let (_fixture, root) = fixture();
        let path = root.join("claims.zip");
        zip_file(&path, |writer| {
            writer
                .start_file(
                    "huge",
                    SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
                )
                .unwrap();
        });
        // Forge the central directory's uncompressed size: 2 GiB from nothing.
        let mut bytes = fs::read(&path).unwrap();
        let central = bytes
            .windows(4)
            .position(|window| window == [0x50, 0x4b, 0x01, 0x02])
            .unwrap();
        bytes[central + 24..central + 28].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
        fs::write(&path, bytes).unwrap();
        let error = extract_into(&path, &root).unwrap_err();
        assert!(error.contains("claims to expand"), "{error}");
        assert_eq!(names(&root), ["claims.zip"]);
    }

    #[test]
    fn mac_resource_forks_do_not_wrap_a_single_folder() {
        let (_fixture, root) = fixture();
        zip_file(&root.join("Photos.zip"), |writer| {
            write_entry(writer, "Photos/one.jpg", b"one");
            write_entry(writer, "__MACOSX/Photos/._one.jpg", b"resource fork");
        });
        assert_eq!(
            extract_into(&root.join("Photos.zip"), &root).unwrap(),
            "Extracted Photos"
        );
        assert_eq!(names(&root), ["Photos", "Photos.zip"]);
        assert_eq!(names(&root.join("Photos")), ["one.jpg"]);
    }

    #[test]
    fn compression_never_replaces_or_contains_its_destination() {
        let (_fixture, root) = fixture();
        fs::create_dir_all(root.join("folder/inner")).unwrap();
        fs::write(root.join("existing.zip"), b"keep me").unwrap();
        assert!(
            execute(Operation::Compress {
                sources: vec![root.join("folder")],
                destination: root.clone(),
                name: "existing.zip".into(),
            })
            .is_err()
        );
        assert_eq!(fs::read(root.join("existing.zip")).unwrap(), b"keep me");
        assert!(
            execute(Operation::Compress {
                sources: vec![root.join("folder")],
                destination: root.join("folder/inner"),
                name: "inside.zip".into(),
            })
            .is_err()
        );
        assert!(names(&root.join("folder/inner")).is_empty());
        assert_eq!(names(&root), ["existing.zip", "folder"]);
    }

    #[test]
    fn numbered_names_keep_file_extensions() {
        assert_eq!(numbered(OsStr::new("a.txt"), 1, false), "a.txt");
        assert_eq!(numbered(OsStr::new("a.txt"), 2, false), "a 2.txt");
        assert_eq!(numbered(OsStr::new("v1.2"), 3, true), "v1.2 3");
        assert_eq!(numbered(OsStr::new(".hidden"), 2, false), ".hidden 2");
    }
}
