//! Measure actual metadata, filtering and sorting of a generated directory.
//! Run: cargo run --locked --release -p finder-core --example scan_benchmark
//! Optional arguments: number of entries (default 10,000), scans (default 11).
//! Fixture creation is excluded from the measured time. OS caches are not reset.

use finder_core::{ScanOptions, scan_directory};
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

struct Fixture(PathBuf);

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let entries: usize = arguments
        .next()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(10_000);
    let iterations: usize = arguments
        .next()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(11);
    if !(1..=100_000).contains(&entries) || !(2..=1_000).contains(&iterations) {
        return Err("use 1..=100000 entries and 2..=1000 iterations".into());
    }
    let unique = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)?
        .as_nanos();
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "neptune-finder-benchmark-{}-{unique}",
        std::process::id()
    )));
    fs::create_dir(&fixture.0)?;
    for index in (0..entries).rev() {
        fs::write(
            fixture.0.join(format!("document-{index}.txt")),
            b"Neptune Finder\n",
        )?;
    }
    let options = ScanOptions::default();
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let listing = scan_directory(&fixture.0, &options)?;
        times.push(started.elapsed().as_secs_f64() * 1_000.0);
        assert_eq!(listing.entries.len(), entries);
        assert!(!listing.truncated);
        std::hint::black_box(listing);
    }
    let first = times.remove(0);
    times.sort_by(f64::total_cmp);
    let percentile =
        |fraction: f64| -> f64 { times[((times.len() - 1) as f64 * fraction).ceil() as usize] };
    println!(
        "host_os={} host_arch={} entries={entries} measured_scans={iterations}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    println!(
        "first_scan_ms={first:.3} warm_p50_ms={:.3} warm_p95_ms={:.3}",
        percentile(0.50),
        percentile(0.95)
    );
    println!(
        "Measures filesystem enumeration, metadata and natural name sorting; fixture setup excluded; OS caches not reset."
    );
    Ok(())
}
