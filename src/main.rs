#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use neptune_finder::{
    Launch,
    app::FinderApp,
    config::Theme,
    persistence::{self, WindowLoad, WindowState},
};
use std::{path::PathBuf, time::Duration};
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
                    "Neptune Finder — a native GPU file explorer\n\nUsage: neptune-finder [PATH] [OPTIONS]\n  --path DIR           Open a directory\n  --data-root DIR      Isolate settings, session and window size\n  --config FILE        Use a TOML configuration\n  --theme THEME        graphite, dusk, light\n  --no-restore         Start a fresh session\n  --size WIDTHxHEIGHT  Open at this size, not the saved one\n  --screenshot FILE    Capture the real native window and exit\n  --benchmark DIR     Measure a directory scan without a GUI\n  --version\n  --help"
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
    // Captures ignore the saved window and never replace it.
    let mut window = if launch.screenshot.is_some() {
        WindowLoad::default()
    } else {
        persistence::restore_window(&launch.data_root(), Duration::from_millis(250))
    };
    if let Some(size) = launch.size {
        window.state.inner_size = size;
        window.state.maximized = false;
    }
    let mut options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Neptune Finder")
            .with_inner_size(window.state.inner_size)
            .with_min_inner_size(WindowState::MIN_SIZE)
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
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration::default().with_surface_config(
            eframe::egui_wgpu::SurfaceConfig {
                // Match Terminal: NVIDIA's FIFO acquire can block the UI thread
                // for seconds after an idle/covered window on Linux hybrid
                // graphics. The automatic mode uses Immediate/Mailbox where
                // supported and retains FIFO as a fallback, without assuming
                // Mailbox exists.
                present_mode: if cfg!(target_os = "linux") {
                    eframe::wgpu::PresentMode::AutoNoVsync
                } else {
                    eframe::wgpu::PresentMode::AutoVsync
                },
                ..eframe::egui_wgpu::SurfaceConfig::LOW_LATENCY
            },
        ),
        ..Default::default()
    };
    // Ask only to maximize. A window that was not maximized opens as it always
    // has, without an unmaximize request as it appears.
    options.viewport.maximized = window.state.maximized.then_some(true);
    eframe::run_native(
        "Neptune Finder",
        options,
        Box::new(move |cc| Ok(Box::new(FinderApp::new(cc, launch, window)))),
    )
    .map_err(|e| e.to_string())
}
