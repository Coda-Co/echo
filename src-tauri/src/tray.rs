//! Tray / menu bar icon and menu. The icon is drawn at runtime so each state
//! (idle, syncing, unseen activity, blocked, error) gets its own badge.

use crate::{show_main, AppState};
use echo_core::{TrayIconState, TrayModel};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

const TRAY_ID: &str = "echo";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let (icon, template) = draw_icon(TrayIconState::Idle);
    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(template)
        .tooltip("Echo")
        .menu(&build_menu(app, &TrayModel { icon: TrayIconState::Idle, synced: vec![], unseen: vec![] })?)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            // Windows convention: left-click opens the window, right-click shows the menu.
            if cfg!(windows) {
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } =
                    event
                {
                    show_main(tray.app_handle());
                }
            }
        });
    let builder = if cfg!(windows) { builder.show_menu_on_left_click(false) } else { builder };
    builder.build(app)?;
    refresh(app);
    Ok(())
}

/// Debounces tray rebuilds and skips them when nothing visible changed.
pub struct TrayRefresher {
    app: AppHandle,
    pending: Arc<AtomicBool>,
}

impl TrayRefresher {
    pub fn new(app: AppHandle) -> Self {
        Self { app, pending: Arc::new(AtomicBool::new(false)) }
    }

    pub fn request(&self) {
        if self.pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let (app, pending) = (self.app.clone(), self.pending.clone());
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            pending.store(false, Ordering::SeqCst);
            let app2 = app.clone();
            let _ = app.run_on_main_thread(move || refresh(&app2));
        });
    }
}

static LAST: Mutex<Option<TrayModel>> = Mutex::new(None);

fn refresh(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else { return };
    let model = state.core.tray_model();
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if last.as_ref() == Some(&model) {
            return;
        }
        *last = Some(model.clone());
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let (icon, template) = draw_icon(model.icon);
    let _ = tray.set_icon(Some(icon));
    let _ = tray.set_icon_as_template(template);
    let tooltip = match model.synced.first() {
        Some(s) => format!("Echo · {} · {}", s.branch, s.description),
        None => "Echo".into(),
    };
    let _ = tray.set_tooltip(Some(tooltip));
    match build_menu(app, &model) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => log::warn!("tray menu: {e}"),
    }
}

fn build_menu(app: &AppHandle, model: &TrayModel) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    let item = |id: &str, text: &str, enabled: bool| MenuItem::with_id(app, id, text, enabled, None::<&str>);

    if model.synced.is_empty() {
        menu.append(&item("none", "Not syncing", false)?)?;
    }
    for s in &model.synced {
        menu.append(&item(&format!("info:{}", s.repo_id), &format!("{} · {}", s.repo_name, s.branch), false)?)?;
        menu.append(&item(&format!("state:{}", s.repo_id), &format!("    {}", capitalise(&s.description)), false)?)?;
        if s.paused {
            menu.append(&item(&format!("resume:{}", s.repo_id), "    Resume", true)?)?;
        } else {
            menu.append(&item(&format!("pause:{}", s.repo_id), "    Pause", true)?)?;
        }
    }
    if !model.unseen.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&item("unseen", "New activity (click to sync)", false)?)?;
        let multi_repo = model.unseen.iter().any(|u| u.repo_id != model.unseen[0].repo_id);
        for u in &model.unseen {
            let label = if multi_repo {
                format!("{} › {}  ({})", u.repo_name, u.branch, u.label)
            } else {
                format!("{}  ({})", u.branch, u.label)
            };
            menu.append(&item(&format!("sync:{}:{}", u.repo_id, u.branch), &label, true)?)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&item("open", "Open Echo", true)?)?;
    menu.append(&item("quit", "Quit Echo", true)?)?;
    Ok(menu)
}

fn capitalise(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id().as_ref().to_string();
    match id.as_str() {
        "open" => show_main(app),
        "quit" => app.exit(0),
        _ => {
            let Some(state) = app.try_state::<AppState>() else { return };
            let core = state.core.clone();
            let app = app.clone();
            let mut parts = id.splitn(3, ':');
            let (action, repo, branch) = (parts.next(), parts.next(), parts.next());
            let (Some(action), Some(repo)) = (action, repo) else { return };
            let (action, repo, branch) = (action.to_string(), repo.to_string(), branch.map(String::from));
            tauri::async_runtime::spawn(async move {
                let r = match (action.as_str(), branch) {
                    ("pause", _) => core.pause_sync(&repo).map(|_| ()),
                    ("resume", _) => core.resume_sync(&repo).await.map(|_| ()),
                    ("sync", Some(b)) => core.start_sync(&repo, &b, false).await.map(|_| ()),
                    _ => Ok(()),
                };
                if let Err(e) = r {
                    // Blocked switches need a decision in the window.
                    log::warn!("tray action {action} failed: {}", e.message);
                    *app.state::<AppState>().pending_focus.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some((repo.clone(), std::time::Instant::now()));
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || show_main(&app2));
                }
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Icon drawing
// ---------------------------------------------------------------------------

const SIZE: u32 = 44;

/// Returns the icon and whether it should be a macOS template image.
pub fn draw_icon(state: TrayIconState) -> (Image<'static>, bool) {
    let badge: Option<[u8; 3]> = match state {
        TrayIconState::Idle => None,
        TrayIconState::Syncing => Some([46, 160, 67]),
        TrayIconState::Unseen => Some([31, 111, 235]),
        TrayIconState::Blocked => Some([210, 153, 34]),
        TrayIconState::Error => Some([218, 54, 51]),
    };
    // macOS: monochrome template when idle (adapts to light/dark menu bars).
    // Elsewhere, and with a coloured badge, use a mid grey that reads on both.
    let template = cfg!(target_os = "macos") && badge.is_none();
    let glyph: [u8; 3] = if template { [0, 0, 0] } else { [128, 128, 128] };

    let s = SIZE as f32;
    let (cx, cy) = (s / 2.0, s / 2.0);
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            // Centre dot + two "echo" rings.
            let mut a = cover(5.0 - d).max(ring(d, 11.0, 2.2)).max(ring(d, 17.5, 2.0) * 0.7);
            let mut colour = glyph;
            if let Some(b) = badge {
                let (bx, by, br) = (s - 9.0, s - 9.0, 8.0);
                let bd = ((px - bx).powi(2) + (py - by).powi(2)).sqrt();
                // Knock out a gap around the badge, then draw it.
                a *= 1.0 - cover(br + 2.5 - bd);
                let ba = cover(br - bd);
                if ba > 0.0 {
                    colour = b;
                    a = a.max(ba);
                }
            }
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i..i + 3].copy_from_slice(&colour);
            rgba[i + 3] = (a.clamp(0.0, 1.0) * 255.0) as u8;
        }
    }
    (Image::new_owned(rgba, SIZE, SIZE), template)
}

/// Antialiased coverage for a signed distance (positive = inside).
fn cover(signed: f32) -> f32 {
    (signed + 0.5).clamp(0.0, 1.0)
}

fn ring(d: f32, radius: f32, half_width: f32) -> f32 {
    cover(half_width - (d - radius).abs())
}
