use super::*;
use std::cell::Cell;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let unique = TEMP_COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let epoch = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "neptune-finder-core-{}-{epoch}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }

    fn file(&self, name: &str, contents: &[u8]) {
        fs::write(self.0.join(name), contents).unwrap();
    }

    fn directory(&self, name: &str) {
        fs::create_dir_all(self.0.join(name)).unwrap();
    }

    fn scan(&self, options: &ScanOptions) -> Listing {
        scan_directory(&self.0, options).unwrap()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn names(listing: &Listing) -> Vec<&str> {
    listing
        .entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect()
}

fn result_for(scanner: &Scanner, generation: u64) -> ScanResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(result) = scanner.poll() {
            assert_eq!(result.generation, generation, "superseded listing escaped");
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "filesystem worker did not finish"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn listing_preserves_native_paths_and_reports_file_metadata() {
    let root = TestDirectory::new();
    root.file("notes.txt", b"neptune");
    root.directory("Documents");
    let listing = root.scan(&ScanOptions::default());
    assert_eq!(listing.path, root.0);
    assert_eq!(names(&listing), ["Documents", "notes.txt"]);
    assert!(!listing.truncated);
    assert!(listing.entries[0].is_dir());
    assert_eq!(listing.entries[0].kind_label(), "Folder");
    assert_eq!(listing.entries[1].path, root.0.join("notes.txt"));
    assert_eq!(listing.entries[1].size, 7);
    assert!(listing.entries[1].modified.is_some());
}

#[test]
fn natural_sort_handles_case_numbers_and_numbers_larger_than_u64() {
    let mut names = [
        "file10",
        "file2",
        "FILE1",
        "file01",
        "file0",
        "file00",
        "file100000000000000000000",
        "file99999999999999999999",
    ];
    names.sort_by(|left, right| natural_cmp(left, right));
    assert_eq!(
        names,
        [
            "file0",
            "file00",
            "FILE1",
            "file01",
            "file2",
            "file10",
            "file99999999999999999999",
            "file100000000000000000000",
        ]
    );
}

#[test]
fn directories_stay_first_when_size_or_name_sort_is_descending() {
    let root = TestDirectory::new();
    root.file("small", b"a");
    root.file("large", b"abc");
    root.directory("folder2");
    root.directory("folder10");
    let mut options = ScanOptions {
        sort: SortOrder {
            field: SortField::Size,
            descending: true,
        },
        ..ScanOptions::default()
    };
    assert_eq!(
        names(&root.scan(&options)),
        ["folder2", "folder10", "large", "small"]
    );
    options.sort.field = SortField::Name;
    assert_eq!(
        names(&root.scan(&options)),
        ["folder10", "folder2", "small", "large"]
    );
}

#[test]
fn hidden_directories_are_not_searched_unless_requested() {
    let root = TestDirectory::new();
    root.file(".secret", b"");
    root.directory(".hidden");
    root.file(".hidden/needle.txt", b"");
    root.directory("visible");
    root.file("visible/needle.txt", b"");
    let mut options = ScanOptions {
        query: "NEEDLE".into(),
        recursive: true,
        ..ScanOptions::default()
    };
    let visible = root.scan(&options);
    assert_eq!(visible.entries.len(), 1);
    assert_eq!(visible.entries[0].path, root.0.join("visible/needle.txt"));
    options.show_hidden = true;
    let all = root.scan(&options);
    assert_eq!(all.entries.len(), 2);
    assert!(!all.truncated);
}

#[test]
fn matching_is_case_insensitive_for_unicode_and_does_not_recurse_by_default() {
    let root = TestDirectory::new();
    root.file("ÉTÉ.txt", b"");
    root.directory("nested");
    root.file("nested/été.txt", b"");
    let options = ScanOptions {
        query: "été".into(),
        ..ScanOptions::default()
    };
    assert_eq!(names(&root.scan(&options)), ["ÉTÉ.txt"]);
}

#[test]
fn listing_limit_bounds_retained_entries_and_marks_truncation() {
    let root = TestDirectory::new();
    for index in 0..12 {
        root.file(&format!("file{index}"), b"");
    }
    let budget = ScanBudget {
        results: 3,
        ..ScanBudget::PRODUCTION
    };
    let listing = scan_with_budget(&root.0, &ScanOptions::default(), || false, budget)
        .unwrap()
        .unwrap();
    assert_eq!(listing.entries.len(), 3);
    assert!(listing.truncated);
}

#[test]
fn traversal_limit_bounds_search_even_when_no_names_match() {
    let root = TestDirectory::new();
    for index in 0..12 {
        root.file(&format!("file{index}"), b"");
    }
    let options = ScanOptions {
        query: "no matches".into(),
        recursive: true,
        ..ScanOptions::default()
    };
    let budget = ScanBudget {
        visited: 4,
        ..ScanBudget::PRODUCTION
    };
    let listing = scan_with_budget(&root.0, &options, || false, budget)
        .unwrap()
        .unwrap();
    assert!(listing.entries.is_empty());
    assert!(listing.truncated);
}

#[test]
fn recursion_depth_is_bounded_and_signals_incomplete_search() {
    let root = TestDirectory::new();
    root.directory("one/two/three");
    root.file("one/two/three/needle", b"");
    let options = ScanOptions {
        recursive: true,
        ..ScanOptions::default()
    };
    let budget = ScanBudget {
        depth: 1,
        ..ScanBudget::PRODUCTION
    };
    let listing = scan_with_budget(&root.0, &options, || false, budget)
        .unwrap()
        .unwrap();
    assert_eq!(names(&listing), ["one", "two"]);
    assert!(listing.truncated);
}

#[test]
fn cancellation_discards_partial_results_during_enumeration() {
    let root = TestDirectory::new();
    for index in 0..20 {
        root.file(&format!("file{index}"), b"");
    }
    let checks = Cell::new(0);
    let result = scan_with_cancellation(&root.0, &ScanOptions::default(), || {
        checks.set(checks.get() + 1);
        checks.get() > 4
    });
    assert!(result.is_none());
    assert_eq!(checks.get(), 5);
}

#[test]
fn absent_directory_is_an_error_instead_of_an_empty_listing() {
    let root = TestDirectory::new();
    assert!(scan_directory(&root.0.join("missing"), &ScanOptions::default()).is_err());
    root.file("file", b"");
    assert!(scan_directory(&root.0.join("file"), &ScanOptions::default()).is_err());
}

#[test]
fn worker_coalesces_to_latest_generation_and_wakes_on_completion() {
    let root = TestDirectory::new();
    root.file("match", b"");
    let (sent, received) = std::sync::mpsc::sync_channel(1);
    let scanner = Scanner::new(move || {
        let _ = sent.try_send(());
    });
    let mut generation = 0;
    for index in 0..100 {
        generation = scanner.request(
            root.0.clone(),
            ScanOptions {
                query: format!("superseded-{index}"),
                ..ScanOptions::default()
            },
        );
    }
    let latest = scanner.request(root.0.clone(), ScanOptions::default());
    assert!(latest > generation);
    let result = result_for(&scanner, latest).result.unwrap();
    assert_eq!(names(&result), ["match"]);
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(scanner.poll().is_none());
}

#[test]
fn a_new_request_discards_a_completed_but_unconsumed_listing() {
    let root = TestDirectory::new();
    root.file("file", b"");
    let (sent, received) = std::sync::mpsc::sync_channel(2);
    let scanner = Scanner::new(move || {
        let _ = sent.try_send(());
    });
    scanner.request(root.0.clone(), ScanOptions::default());
    received.recv_timeout(Duration::from_secs(5)).unwrap();
    let latest = scanner.request(root.0.join("missing"), ScanOptions::default());
    assert!(result_for(&scanner, latest).result.is_err());
}

#[test]
fn scanner_drop_does_not_wait_for_a_blocked_wake_callback() {
    let root = TestDirectory::new();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_release = Arc::clone(&release);
    let scanner = Scanner::new(move || {
        let _ = entered_tx.send(());
        let (lock, condition) = &*worker_release;
        let mut done = lock.lock().unwrap();
        while !*done {
            done = condition.wait(done).unwrap();
        }
    });
    scanner.request(root.0.clone(), ScanOptions::default());
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let started = Instant::now();
    drop(scanner);
    let elapsed = started.elapsed();
    let (lock, condition) = &*release;
    *lock.lock().unwrap() = true;
    condition.notify_one();
    assert!(elapsed < Duration::from_secs(1));
}

#[test]
fn retirement_backpressure_returns_ownership_and_does_not_disrupt_scans() {
    let root = TestDirectory::new();
    root.file("notes", b"retire this owned snapshot");
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_release = Arc::clone(&release);
    let scanner = Scanner::new(move || {
        let _ = entered_tx.try_send(());
        let (lock, condition) = &*worker_release;
        let mut done = lock.lock().unwrap();
        while !*done {
            done = condition.wait(done).unwrap();
        }
    });
    let generation = scanner.request(root.0.clone(), ScanOptions::default());
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let completed = scanner.poll().unwrap();
    assert_eq!(completed.generation, generation);
    let entries = completed.result.unwrap().entries;
    let second = entries.clone();
    scanner.retire(entries).unwrap();
    let retained = scanner
        .retire(second)
        .expect_err("retirement slot must remain bounded");
    assert_eq!(retained.len(), 1);
    assert_eq!(
        lock_state(&scanner.shared).retired.as_ref().unwrap().len(),
        1
    );
    // An empty vector never consumes a slot, including under backpressure.
    scanner.retire(Vec::new()).unwrap();
    let (lock, condition) = &*release;
    *lock.lock().unwrap() = true;
    condition.notify_one();
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock_state(&scanner.shared).retired.is_some() {
        assert!(
            Instant::now() < deadline,
            "retirement worker did not release its slot"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    scanner.retire(retained).unwrap();
    let latest = scanner.request(root.0.clone(), ScanOptions::default());
    assert_eq!(
        names(&result_for(&scanner, latest).result.unwrap()),
        ["notes"]
    );
}

#[cfg(unix)]
#[test]
fn explicitly_opened_directory_alias_uses_canonical_operation_targets() {
    use std::os::unix::fs::symlink;
    let root = TestDirectory::new();
    let target = TestDirectory::new();
    target.file("notes", b"");
    let alias = root.0.join("alias");
    symlink(&target.0, &alias).unwrap();
    let listing = scan_directory(&alias, &ScanOptions::default()).unwrap();
    assert_eq!(listing.path, fs::canonicalize(&target.0).unwrap());
    assert_eq!(listing.entries[0].path, listing.path.join("notes"));
}

#[cfg(unix)]
#[test]
fn recursive_search_never_follows_directory_symlinks_or_cycles() {
    use std::os::unix::fs::symlink;
    let root = TestDirectory::new();
    let external = TestDirectory::new();
    external.file("private-needle", b"");
    root.directory("local");
    root.file("local/needle", b"");
    symlink(&external.0, root.0.join("outside")).unwrap();
    symlink(&root.0, root.0.join("local/cycle")).unwrap();
    symlink(root.0.join("missing"), root.0.join("broken")).unwrap();
    let all = root.scan(&ScanOptions {
        recursive: true,
        ..ScanOptions::default()
    });
    assert_eq!(all.entries.len(), 5);
    assert!(!all.truncated);
    for name in ["outside", "cycle"] {
        let entry = all.entries.iter().find(|entry| entry.name == name).unwrap();
        assert_eq!(entry.kind, EntryKind::DirectoryLink);
        assert!(entry.is_dir());
    }
    let broken = all
        .entries
        .iter()
        .find(|entry| entry.name == "broken")
        .unwrap();
    assert_eq!(broken.kind, EntryKind::Symlink);
    assert!(!broken.is_dir());
    let outside = all
        .entries
        .iter()
        .find(|entry| entry.name == "outside")
        .unwrap();
    assert_eq!(
        outside.path,
        root.0.join("outside"),
        "preserve the link itself for rename and Trash"
    );
    assert!(
        !all.entries
            .iter()
            .any(|entry| entry.name == "private-needle")
    );
}

#[cfg(unix)]
#[test]
fn non_utf8_display_names_do_not_destroy_native_paths() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let root = TestDirectory::new();
    let native = root.0.join(OsString::from_vec(b"notes-\xff".to_vec()));
    fs::write(&native, b"original").unwrap();
    let listing = root.scan(&ScanOptions::default());
    assert_eq!(listing.entries[0].path, native);
    assert_eq!(fs::read(&listing.entries[0].path).unwrap(), b"original");
}
