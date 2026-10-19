#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use neptune_finder::{Launch, app::FinderApp, config::Theme};
use std::path::PathBuf;
fn main() {
    if let Err(error) = run() {
        eprintln!("Neptune Finder: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut launch = Launch::default();
    let mut benchmark = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let value = arg.to_string_lossy();
        match value.as_ref() {
            "--path" => launch.path = Some(args.next().ok_or("--path needs a directory")?.into()),
            "--data-root" => {
                launch.data_root = Some(args.next().ok_or("--data-root needs a path")?.into())
            }
            "--config" => {
                launch.config = Some(args.next().ok_or("--config needs a TOML path")?.into())
            }
            "--screenshot" => {
                launch.screenshot = Some(args.next().ok_or("--screenshot needs a PNG path")?.into())
            }
            "--benchmark" => {
                benchmark = Some(PathBuf::from(
                    args.next().ok_or("--benchmark needs a directory")?,
                ))
            }
            "--theme" => {
                launch.theme = Some(
                    match args
                        .next()
                        .ok_or("--theme needs graphite, dusk or light")?
                        .to_string_lossy()
                        .as_ref()
                    {
                        "graphite" => Theme::Graphite,
                        "dusk" => Theme::Dusk,
                        "light" => Theme::Light,
                        _ => return Err("unknown theme".into()),
                    },
                )
            }
            "--no-restore" => launch.no_restore = true,
            "--size" => {
                let size = args
                    .next()
                    .ok_or("--size needs WIDTHxHEIGHT")?
                    .to_string_lossy()
                    .into_owned();
                let (w, h) = size.split_once('x').ok_or("--size needs WIDTHxHEIGHT")?;
                let size = [
                    w.parse::<f32>().map_err(|_| "invalid width")?,
                    h.parse::<f32>().map_err(|_| "invalid height")?,
                ];
                if size
                    .iter()
                    .any(|v| !v.is_finite() || *v < 300.0 || *v > 16384.0)
                {
                    return Err("window dimensions must be 300–16384".into());
                }
                launch.size = Some(size);
            }
            "--version" | "-V" => {
                println!("Neptune Finder {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "Neptune Finder — a native GPU file explorer\n\nUsage: neptune-finder [PATH] [OPTIONS]\n  --path DIR           Open a directory\n  --data-root DIR      Isolate settings and saved session\n  --config FILE        Use a TOML configuration\n  --theme THEME        graphite, dusk, light\n  --no-restore         Start a fresh session\n  --size WIDTHxHEIGHT  Initial window size\n  --screenshot FILE    Capture the real native window and exit\n  --benchmark DIR     Measure a directory scan without a GUI\n  --version\n  --help"
                );
                return Ok(());
            }
            _ if !value.starts_with('-') && launch.path.is_none() => launch.path = Some(arg.into()),
            _ => return Err(format!("unknown argument: {value}")),
        }
    }
    if let Some(path) = benchmark {
        let listing = finder_core::scan_directory(
            &path,
            &finder_core::ScanOptions {
                show_hidden: true,
                ..Default::default()
            },
        )?;
        println!(
            "{} entries in {:.3} ms{}",
            listing.entries.len(),
            listing.elapsed.as_secs_f64() * 1000.0,
            if listing.truncated {
                " (truncated)"
            } else {
                ""
            }
        );
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Neptune Finder")
            .with_inner_size(launch.size.unwrap_or([1240.0, 800.0]))
            .with_min_inner_size([520.0, 360.0])
            .with_app_id("rs.neptune.finder")
            .with_decorations(false)
            // Match Terminal: Windows supplies the native border and rounding.
            .with_transparent(!cfg!(target_os = "windows"))
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../assets/branding/neptune-finder-icon.png"
                ))
                .map_err(|e| e.to_string())?,
            ),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "Neptune Finder",
        options,
        Box::new(move |cc| Ok(Box::new(FinderApp::new(cc, launch)))),
    )
    .map_err(|e| e.to_string())
}
