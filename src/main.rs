#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod config;
mod discord;
mod engine;
mod remote;
mod shell;
mod state;
mod ui;
mod usage;
mod util;

use eframe::egui;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use util::log;

static QUITTING: AtomicBool = AtomicBool::new(false);
/// Clear presence, drop the tray icon, free the singleton, optionally start a fresh copy, then leave.
fn quit(eng: &Arc<engine::Engine>, restart: bool) {
    if QUITTING.swap(true, Ordering::SeqCst) { return; }
    shell::hide_main();
    eng.stop();
    shell::remove_tray();
    shell::release_singleton();
    if restart {
        log("restarting");
        if let Ok(exe) = std::env::current_exe() {
            if let Err(e) = std::process::Command::new(exe).arg("--minimized").spawn() { log(&format!("restart failed: {e}")); }
        }
    }
    std::process::exit(0);
}

fn window_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../t3-logo-1024.png")).ok()?;
    let img = img.resize_exact(64, 64, image::imageops::FilterType::Lanczos3).to_rgba8();
    Some(egui::IconData { width: 64, height: 64, rgba: img.into_raw() })
}

fn main() {
    std::panic::set_hook(Box::new(|p| log(&format!("crash: {p}"))));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mut dump, mut minimized, mut settings, mut exit, mut restart, mut drop) = (None::<String>, false, false, false, false, 0u64);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--dump" => { dump = Some(args.get(i + 1).cloned().unwrap_or_else(|| util::path_of("T3Presence.dump.txt").to_string_lossy().into())); i += 1; }
            "--minimized" => minimized = true,
            "--settings" => settings = true,
            "--exit" => exit = true,
            "--restart" => restart = true,
            a if a.starts_with("--test-drop=") => drop = a[12..].parse().unwrap_or(0),
            _ => {}
        }
        i += 1;
    }
    let cfg = config::Config::load();
    if let Some(p) = dump {
        let _ = std::fs::write(p, engine::Engine::new(cfg).dump());
        return;
    }
    if !shell::claim_singleton() {
        let (ev, what) = if exit { ("Exit", "exit") } else if restart { ("Restart", "restart") } else { ("Show", "show itself") };
        shell::signal(ev);
        log(&format!("second launch: asked the running copy to {what}"));
        return;
    }
    if exit || restart { return; }
    shell::refresh_autostart();

    let eng = engine::Engine::new(cfg);
    eng.test_drop_after.store(drop, Ordering::SeqCst);
    {
        let e1 = eng.clone(); let e2 = eng.clone();
        shell::start_tray(shell::TrayHooks {
            on: Box::new(move |c| match c {
                shell::TrayCmd::Show => shell::show_main(),
                shell::TrayCmd::Pause => e1.set_paused(!e1.is_paused()),
                shell::TrayCmd::Autostart => shell::set_autostart(!shell::autostart_enabled()),
                shell::TrayCmd::Restart => { let e = e1.clone(); std::thread::spawn(move || quit(&e, true)); }
                shell::TrayCmd::Exit => { let e = e1.clone(); std::thread::spawn(move || quit(&e, false)); }
            }),
            paused: Box::new(move || e2.is_paused()),
        });
    }
    shell::on_event("Show", shell::show_main);
    { let e = eng.clone(); shell::on_event("Exit", move || quit(&e, false)); }
    { let e = eng.clone(); shell::on_event("Restart", move || quit(&e, true)); }

    let title = if cfg!(feature = "dev") { "T3 Presence (dev)" } else { "T3 Presence" };
    let mut vp = egui::ViewportBuilder::default().with_title(title).with_inner_size([500.0, 500.0]).with_min_inner_size([440.0, 470.0]).with_visible(!minimized);
    if let Some(ic) = window_icon() { vp = vp.with_icon(Arc::new(ic)); }
    let opts = eframe::NativeOptions { viewport: vp, centered: true, ..Default::default() };
    let e = eng.clone();
    let r = eframe::run_native(title, opts, Box::new(move |cc| {
        if let Ok(h) = cc.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                let hwnd = w.hwnd.get();
                shell::MAIN_HWND.store(hwnd, Ordering::SeqCst);
                shell::dark_title(hwnd);
            }
        }
        egui_extras::install_image_loaders(&cc.egui_ctx);
        ui::setup_fonts(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        *e.on_update.lock().unwrap() = Some(Box::new(move || ctx.request_repaint()));
        e.start();
        Ok(Box::new(ui::App::new(e.clone(), settings)))
    }));
    if let Err(err) = r { log(&format!("ui error: {err}")); }
    quit(&eng, false);
}
