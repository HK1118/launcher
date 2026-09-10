use eframe::egui;
use notify::{Event, Watcher};
use std::collections::{HashMap, HashSet};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

#[cfg(windows)]
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

use crate::icon::extract_icon_image;
use crate::models::{
    SavedApp, Settings, get_data_path, load_apps, load_settings, save_apps, save_settings,
};
use crate::platform::{normalize_path_key, shell_open};
use crate::theme;
use crate::views::launcher::render_launcher_screen;
use crate::views::settings::render_settings_screen;
use crate::views::widgets::{render_rename_modal, render_toast};

#[derive(Clone)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CurrentScreen {
    #[default]
    Launcher,
    Settings,
}

#[derive(Clone, Copy)]
pub enum ReloadEvent {
    Apps,
    Settings,
}

pub struct LauncherApp {
    pub apps: Vec<SavedApp>,
    pub settings: Settings,
    pub current_screen: CurrentScreen,
    pub toast: Option<(String, ToastKind, Instant)>,
    pub file_receiver: Receiver<ReloadEvent>,
    pub _watcher: Option<notify::RecommendedWatcher>,
    pub editing_name: Option<(usize, String)>,
    pub launching_apps: HashMap<String, Instant>,
    pub icon_textures: HashMap<String, Option<egui::TextureHandle>>,
    pub icon_req_tx: Sender<String>,
    pub icon_res_rx: Receiver<(String, Option<egui::ColorImage>)>,
    pub requested_icons: HashSet<String>,
    pub file_existence: HashMap<String, bool>,
    pub last_existence_check: Instant,
    pub last_window_focused: bool,
    pub dragging_idx: Option<usize>,
    pub drop_target_idx: Option<usize>,
    pub scroll_offset: f32,
    pub last_apps_saved: Instant,
    pub last_settings_saved: Instant,
}

impl LauncherApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (file_tx, file_rx) = channel();
        let egui_ctx = cc.egui_ctx.clone();

        let apps_path = get_data_path("apps.json");
        let watch_dir = apps_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                for path in &event.paths {
                    if path.ends_with("apps.json") {
                        let _ = file_tx.send(ReloadEvent::Apps);
                        egui_ctx.request_repaint();
                    } else if path.ends_with("settings.json") {
                        let _ = file_tx.send(ReloadEvent::Settings);
                        egui_ctx.request_repaint();
                    }
                }
            }
        })
        .ok();

        if let Some(ref mut w) = watcher {
            let _ = w.watch(&watch_dir, notify::RecursiveMode::NonRecursive);
        }

        let (settings, settings_warn) = load_settings();
        let (apps, app_err) = match load_apps() {
            Ok(apps) => (apps, None),
            Err(err) => (Vec::new(), Some(err)),
        };

        let initial_toast = app_err
            .map(|e| (e, ToastKind::Error, Instant::now()))
            .or_else(|| settings_warn.map(|w| (w, ToastKind::Error, Instant::now())));

        let (req_tx, req_rx) = channel::<String>();
        let (res_tx, res_rx) = channel::<(String, Option<egui::ColorImage>)>();
        let bg_ctx = cc.egui_ctx.clone();

        std::thread::spawn(move || {
            #[cfg(windows)]
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }

            while let Ok(path) = req_rx.recv() {
                let img = extract_icon_image(&path);
                let _ = res_tx.send((path, img));
                bg_ctx.request_repaint();
            }

            #[cfg(windows)]
            unsafe {
                CoUninitialize();
            }
        });

        let past = Instant::now() - Duration::from_secs(60);

        let mut app_state = Self {
            apps,
            settings,
            current_screen: CurrentScreen::Launcher,
            toast: initial_toast,
            file_receiver: file_rx,
            _watcher: watcher,
            editing_name: None,
            launching_apps: HashMap::new(),
            icon_textures: HashMap::new(),
            icon_req_tx: req_tx,
            icon_res_rx: res_rx,
            requested_icons: HashSet::new(),
            file_existence: HashMap::new(),
            last_existence_check: Instant::now(),
            last_window_focused: true,
            dragging_idx: None,
            drop_target_idx: None,
            scroll_offset: 0.0,
            last_apps_saved: past,
            last_settings_saved: past,
        };

        app_state.refresh_file_existence();
        app_state
    }

    pub fn save_apps_internal(&mut self) {
        self.last_apps_saved = Instant::now();
        save_apps(&self.apps);
    }

    pub fn save_settings_internal(&mut self) {
        self.last_settings_saved = Instant::now();
        save_settings(&self.settings);
    }

    pub fn refresh_file_existence(&mut self) {
        self.file_existence.clear();
        for app in &self.apps {
            self.file_existence
                .insert(app.path.clone(), Path::new(&app.path).exists());
        }
        self.last_existence_check = Instant::now();
    }

    pub fn add_paths(&mut self, paths: &[PathBuf]) {
        let allowed = ["exe", "lnk", "url", "html", "htm"];
        let mut added_count = 0;
        let mut skipped_count = 0;
        let mut unsupported_count = 0;

        let mut existing_keys: HashSet<String> = self
            .apps
            .iter()
            .map(|a| normalize_path_key(&a.path))
            .collect();

        for path in paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();

            if !allowed.contains(&ext.as_str()) {
                unsupported_count += 1;
                continue;
            }

            let path_str = path.to_string_lossy().to_string();
            let norm_key = normalize_path_key(&path_str);

            if existing_keys.contains(&norm_key) {
                skipped_count += 1;
                continue;
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();

            existing_keys.insert(norm_key);
            self.apps.push(SavedApp {
                name,
                path: path_str,
            });
            added_count += 1;
        }

        if added_count > 0 {
            self.save_apps_internal();
            self.refresh_file_existence();

            let msg = if skipped_count > 0 {
                format!(
                    "{} 件のアプリを追加しました ({} 件は重複のためスキップ)",
                    added_count, skipped_count
                )
            } else {
                format!("{} 件のアプリを追加しました", added_count)
            };
            self.toast = Some((msg, ToastKind::Info, Instant::now()));
        } else if skipped_count > 0 {
            self.toast = Some((
                format!(
                    "選択された {} 件のアプリは既に登録されています",
                    skipped_count
                ),
                ToastKind::Info,
                Instant::now(),
            ));
        } else if unsupported_count > 0 {
            self.toast = Some((
                "対応していないファイル形式です (.exe, .lnk, .url, .html)".to_string(),
                ToastKind::Error,
                Instant::now(),
            ));
        }
    }

    pub fn launch(&mut self, app_name: &str, path_str: &str) {
        let cooldown = Duration::from_secs(self.settings.launch_cooldown_secs);

        if let Some(start) = self.launching_apps.get(path_str)
            && start.elapsed() < cooldown
        {
            return;
        }

        let target_path = Path::new(path_str);

        if !target_path.exists() {
            self.file_existence.insert(path_str.to_string(), false);
            self.toast = Some((
                format!("「{}」のファイルが見つかりません:\n{}", app_name, path_str),
                ToastKind::Error,
                Instant::now(),
            ));
            return;
        }

        let working_dir = target_path.parent().filter(|p| p.exists() && p.is_dir());

        match shell_open(path_str, working_dir) {
            Ok(_) => {
                self.launching_apps
                    .insert(path_str.to_string(), Instant::now());
                self.toast = Some((
                    format!("「{}」を起動しています...", app_name),
                    ToastKind::Info,
                    Instant::now(),
                ));
            }
            Err(err) => {
                let msg = format!("{} を起動できませんでした: {}", path_str, err);
                self.toast = Some((msg, ToastKind::Error, Instant::now()));
            }
        }
    }

    pub fn delete_app(&mut self, idx: usize) {
        if idx >= self.apps.len() {
            return;
        }

        if self.settings.confirm_on_delete {
            let app_name = &self.apps[idx].name;
            let confirm = rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("削除の確認")
                .set_description(format!(
                    "「{}」を一覧から削除してもよろしいですか？",
                    app_name
                ))
                .set_buttons(rfd::MessageButtons::YesNo)
                .show();

            if confirm != rfd::MessageDialogResult::Yes {
                return;
            }
        }

        let removed = self.apps.remove(idx);
        self.icon_textures.remove(&removed.path);
        self.requested_icons.remove(&removed.path);
        self.file_existence.remove(&removed.path);
        self.dragging_idx = None;
        self.drop_target_idx = None;
        self.save_apps_internal();
    }

    pub fn open_location(&mut self, path_str: &str) {
        let target = Path::new(path_str);
        if target.exists() {
            if target.is_dir() {
                let _ = std::process::Command::new("explorer").arg(target).spawn();
            } else {
                let _ = std::process::Command::new("explorer")
                    .raw_arg(format!("/select,\"{}\"", target.to_string_lossy()))
                    .spawn();
            }
        } else if let Some(parent) = target.parent()
            && parent.exists()
        {
            let _ = std::process::Command::new("explorer").arg(parent).spawn();
        } else {
            self.toast = Some((
                format!("保存場所が見つかりません:\n{}", path_str),
                ToastKind::Error,
                Instant::now(),
            ));
        }
    }

    pub fn open_file_with_default_app(path: &Path) {
        let path_str = path.to_string_lossy();
        let _ = shell_open(&path_str, None);
    }
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = theme::BG_APP;
        ui.ctx().set_visuals(visuals);

        let cooldown = Duration::from_secs(self.settings.launch_cooldown_secs);

        self.launching_apps
            .retain(|_, t| t.elapsed() < cooldown + Duration::from_secs(5));

        while let Ok((path, img_opt)) = self.icon_res_rx.try_recv() {
            let tex = img_opt.map(|img| {
                ui.ctx()
                    .load_texture(format!("icon_{}", path), img, egui::TextureOptions::LINEAR)
            });
            self.icon_textures.insert(path, tex);
        }

        // 外部変更検知によるホットリロード（自前保存直後ならスキップ）
        while let Ok(event) = self.file_receiver.try_recv() {
            match event {
                ReloadEvent::Apps => {
                    if self.last_apps_saved.elapsed() >= Duration::from_millis(600)
                        && let Ok(loaded) = load_apps()
                        && loaded != self.apps
                    {
                        self.apps = loaded;
                        self.refresh_file_existence();
                    }
                }
                ReloadEvent::Settings => {
                    if self.last_settings_saved.elapsed() >= Duration::from_millis(600) {
                        let (loaded, warn) = load_settings();
                        self.settings = loaded;
                        if let Some(w) = warn {
                            self.toast = Some((w, ToastKind::Error, Instant::now()));
                        }
                    }
                }
            }
        }

        let is_focused = ui.input(|i| i.focused);
        if (is_focused && !self.last_window_focused)
            || self.last_existence_check.elapsed() > Duration::from_secs(5)
        {
            self.refresh_file_existence();
        }
        self.last_window_focused = is_focused;

        // メイン画面での外部ファイルドロップ受付（編集モード中のみ受け付ける）
        if self.current_screen == CurrentScreen::Launcher {
            if self.settings.edit_mode {
                ui.ctx().input(|i| {
                    if !i.raw.dropped_files.is_empty() {
                        let paths: Vec<PathBuf> = i
                            .raw
                            .dropped_files
                            .iter()
                            .map(|d| d.path().to_path_buf())
                            .filter(|p| !p.as_os_str().is_empty())
                            .collect();
                        self.add_paths(&paths);
                    }
                });
            } else {
                // 編集モードがオフのときにドロップされたら案内を表示
                let has_dropped = ui.ctx().input(|i| !i.raw.dropped_files.is_empty());
                if has_dropped {
                    self.toast = Some((
                        "アプリを追加するには右上の「編集」ボタンを押してください".to_string(),
                        ToastKind::Info,
                        Instant::now(),
                    ));
                }
            }

            if self.dragging_idx.is_some() {
                let scroll_delta_y = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll_delta_y != 0.0 {
                    self.scroll_offset = (self.scroll_offset - scroll_delta_y).max(0.0);
                }
            }
        }

        let panel_frame = egui::Frame::new()
            .fill(theme::BG_APP)
            .inner_margin(egui::Margin::symmetric(20, 16));

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ui, |ui| match self.current_screen {
                CurrentScreen::Launcher => render_launcher_screen(self, ui, cooldown),
                CurrentScreen::Settings => render_settings_screen(self, ui),
            });

        if self.launching_apps.values().any(|t| t.elapsed() < cooldown) {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }

        render_rename_modal(self, ui.ctx());
        render_toast(self, ui.ctx());
    }
}
