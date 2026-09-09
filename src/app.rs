use eframe::egui;
use notify::{Event, Watcher};
use rfd::FileDialog;
use std::collections::{HashMap, HashSet};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::icon::extract_icon_image;
use crate::models::{
    SavedApp, Settings, get_data_path, load_apps, load_settings, save_apps, save_settings,
};

#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(pvReserved: *mut std::ffi::c_void, dwCoInit: u32) -> i32;
    fn CoUninitialize();
}

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
enum ReloadEvent {
    Apps,
    Settings,
}

pub struct LauncherApp {
    apps: Vec<SavedApp>,
    settings: Settings,
    current_screen: CurrentScreen,
    toast: Option<(String, ToastKind, Instant)>,
    file_receiver: Receiver<ReloadEvent>,
    _watcher: Option<notify::RecommendedWatcher>,
    editing_name: Option<(usize, String)>,
    launching_apps: HashMap<String, Instant>,
    icon_textures: HashMap<String, Option<egui::TextureHandle>>,
    icon_req_tx: Sender<String>,
    icon_res_rx: Receiver<(String, Option<egui::ColorImage>)>,
    requested_icons: HashSet<String>,
    file_existence: HashMap<String, bool>,
    last_existence_check: Instant,
    last_window_focused: bool,
    dragging_idx: Option<usize>,
    drop_target_idx: Option<usize>,
    scroll_offset: f32,
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

        // アイコン抽出のバックグラウンドワーカー（COM初期化）
        let (req_tx, req_rx) = channel::<String>();
        let (res_tx, res_rx) = channel::<(String, Option<egui::ColorImage>)>();
        let bg_ctx = cc.egui_ctx.clone();

        std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(std::ptr::null_mut(), 2);
            }

            while let Ok(path) = req_rx.recv() {
                let img = extract_icon_image(&path);
                let _ = res_tx.send((path, img));
                bg_ctx.request_repaint();
            }

            unsafe {
                CoUninitialize();
            }
        });

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
        };

        app_state.refresh_file_existence();
        app_state
    }

    fn refresh_file_existence(&mut self) {
        self.file_existence.clear();
        for app in &self.apps {
            self.file_existence
                .insert(app.path.clone(), Path::new(&app.path).exists());
        }
        self.last_existence_check = Instant::now();
    }

    fn add_paths(&mut self, paths: &[PathBuf]) {
        let allowed = ["exe", "lnk", "url", "html", "htm"];
        let mut added = false;

        for path in paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_lowercase())
                .unwrap_or_default();
            if !allowed.contains(&ext.as_str()) {
                continue;
            }

            let path_str = path.to_string_lossy().to_string();
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();

            if self.apps.iter().any(|app| app.path == path_str) {
                continue;
            }

            self.apps.push(SavedApp {
                name,
                path: path_str,
            });
            added = true;
        }

        if added {
            save_apps(&self.apps);
            self.refresh_file_existence();
        }
    }

    fn launch(&mut self, app_name: &str, path_str: &str) {
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

        let ext = target_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let mut cmd = if ext == "lnk" || ext == "url" || ext == "html" || ext == "htm" {
            let mut c = std::process::Command::new("cmd");
            c.args(["/c", "start", "", path_str]);
            c.creation_flags(0x08000000);
            c
        } else {
            std::process::Command::new(target_path)
        };

        if let Some(parent) = target_path.parent()
            && parent.exists()
            && parent.is_dir()
        {
            cmd.current_dir(parent);
        }

        match cmd.spawn() {
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

    fn delete_app(&mut self, idx: usize) {
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
        save_apps(&self.apps);
    }

    fn open_location(&mut self, path_str: &str) {
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

    fn open_file_with_default_app(path: &Path) {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "start", "", &path.to_string_lossy()]);
        cmd.creation_flags(0x08000000);
        let _ = cmd.spawn();
    }
}

fn custom_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    text: &str,
    base_bg: egui::Color32,
    fg: egui::Color32,
    enabled: bool,
) -> egui::Response {
    let resp = ui.interact(
        rect,
        id,
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );

    let (bg, text_color) = if !enabled {
        (
            egui::Color32::from_rgb(247, 250, 252),
            egui::Color32::from_rgb(203, 213, 225),
        )
    } else if resp.is_pointer_button_down_on() {
        (
            egui::Color32::from_rgba_premultiplied(
                (base_bg.r() as f32 * 0.85) as u8,
                (base_bg.g() as f32 * 0.85) as u8,
                (base_bg.b() as f32 * 0.85) as u8,
                255,
            ),
            fg,
        )
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        (
            egui::Color32::from_rgba_premultiplied(
                (base_bg.r() as f32 * 1.1).min(255.0) as u8,
                (base_bg.g() as f32 * 1.1).min(255.0) as u8,
                (base_bg.b() as f32 * 1.1).min(255.0) as u8,
                255,
            ),
            fg,
        )
    } else {
        (base_bg, fg)
    };

    let font_size = if text.chars().count() == 1 {
        11.0
    } else {
        12.0
    };

    ui.painter().rect(
        rect,
        4.0,
        bg,
        egui::Stroke::new(1.0, egui::Color32::from_rgb(226, 232, 240)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(font_size),
        text_color,
    );

    resp
}

impl eframe::App for LauncherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = egui::Color32::from_rgb(247, 250, 252);
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

        // ファイル変更検知によるホットリロード
        while let Ok(event) = self.file_receiver.try_recv() {
            match event {
                ReloadEvent::Apps => {
                    if let Ok(loaded) = load_apps()
                        && loaded != self.apps
                    {
                        self.apps = loaded;
                        self.refresh_file_existence();
                    }
                }
                ReloadEvent::Settings => {
                    let (loaded, warn) = load_settings();
                    self.settings = loaded;
                    if let Some(w) = warn {
                        self.toast = Some((w, ToastKind::Error, Instant::now()));
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

        // メイン画面でのみファイルの外部ドロップを受け付ける
        if self.current_screen == CurrentScreen::Launcher {
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

            // ドラッグ中のマウスホイールによるスクロール処理
            if self.dragging_idx.is_some() {
                let scroll_delta_y = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll_delta_y != 0.0 {
                    self.scroll_offset = (self.scroll_offset - scroll_delta_y).max(0.0);
                }
            }
        }

        let panel_frame = egui::Frame::new()
            .fill(egui::Color32::from_rgb(247, 250, 252))
            .inner_margin(egui::Margin::symmetric(20, 16));

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ui, |ui| match self.current_screen {
                CurrentScreen::Launcher => self.render_launcher_screen(ui, cooldown),
                CurrentScreen::Settings => self.render_settings_screen(ui),
            });

        if self.launching_apps.values().any(|t| t.elapsed() < cooldown) {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }

        // 名前の変更モーダル
        if let Some((idx, ref mut name_buf)) = self.editing_name {
            let mut save_clicked = false;
            let mut close_clicked = false;

            egui::Window::new("名前の変更")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .fixed_size(egui::vec2(320.0, 120.0))
                .show(ui.ctx(), |ui| {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new("新しいアプリケーション名を入力してください:")
                            .size(13.0),
                    );
                    ui.add_space(8.0);

                    let text_resp =
                        ui.add(egui::TextEdit::singleline(name_buf).desired_width(f32::INFINITY));
                    if text_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        save_clicked = true;
                    }

                    ui.add_space(14.0);

                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add_sized(egui::vec2(70.0, 28.0), egui::Button::new("保存"))
                                .on_hover_text("変更を保存 (Enter)")
                                .clicked()
                            {
                                save_clicked = true;
                            }
                            ui.add_space(8.0);
                            if ui
                                .add_sized(egui::vec2(70.0, 28.0), egui::Button::new("キャンセル"))
                                .on_hover_text("変更を破棄して閉じる (Esc)")
                                .clicked()
                            {
                                close_clicked = true;
                            }
                        });
                    });

                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close_clicked = true;
                    }
                });

            if save_clicked {
                let new_name = name_buf.trim().to_string();
                if !new_name.is_empty() && idx < self.apps.len() {
                    self.apps[idx].name = new_name;
                    save_apps(&self.apps);
                }
                self.editing_name = None;
            } else if close_clicked {
                self.editing_name = None;
            }
        }

        // 通知トースト
        if let Some((ref msg, ref kind, start_time)) = self.toast {
            let timeout = match kind {
                ToastKind::Info => Duration::from_secs(4),
                ToastKind::Error => Duration::from_secs(6),
            };

            if start_time.elapsed() > timeout {
                self.toast = None;
            } else {
                let msg_clone = msg.clone();
                let (tag, bg, fg) = match kind {
                    ToastKind::Info => (
                        "INFO",
                        egui::Color32::from_rgb(44, 122, 123),
                        egui::Color32::WHITE,
                    ),
                    ToastKind::Error => (
                        "WARN",
                        egui::Color32::from_rgb(197, 48, 48),
                        egui::Color32::WHITE,
                    ),
                };

                egui::Area::new(egui::Id::new("toast_notification"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -25.0))
                    .show(ui.ctx(), |ui| {
                        let frame = egui::Frame::new()
                            .fill(bg)
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(16, 10));

                        let resp = frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(tag).color(fg).size(15.0));
                                ui.label(
                                    egui::RichText::new(&msg_clone)
                                        .color(egui::Color32::WHITE)
                                        .size(13.0),
                                );
                            });
                        });

                        if resp.response.interact(egui::Sense::click()).clicked() {
                            self.toast = None;
                        }
                    });
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            }
        }
    }
}

impl LauncherApp {
    // ----------------------------------------------------
    // メイン画面（ランチャー）の描画
    // ----------------------------------------------------
    fn render_launcher_screen(&mut self, ui: &mut egui::Ui, cooldown: Duration) {
        ui.horizontal(|ui| {
            ui.heading(
                egui::RichText::new("ゲームランチャー")
                    .color(egui::Color32::from_rgb(45, 55, 72))
                    .size(22.0)
                    .strong(),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // 1. 編集モード切り替えボタン（常に右端に固定表示）
                let (edit_label, edit_bg, edit_fg) = if self.settings.edit_mode {
                    (
                        "完了",
                        egui::Color32::from_rgb(66, 153, 225),
                        egui::Color32::WHITE,
                    )
                } else {
                    (
                        "編集",
                        egui::Color32::from_rgb(237, 242, 247),
                        egui::Color32::from_rgb(74, 85, 104),
                    )
                };

                let edit_btn =
                    egui::Button::new(egui::RichText::new(edit_label).color(edit_fg).size(13.0))
                        .fill(edit_bg)
                        .corner_radius(4.0);

                if ui
                    .add_sized(egui::vec2(68.0, 30.0), edit_btn)
                    .on_hover_text(if self.settings.edit_mode {
                        "編集モードを終了して固定"
                    } else {
                        "アプリの並び替え・追加・設定を行う"
                    })
                    .clicked()
                {
                    self.settings.edit_mode = !self.settings.edit_mode;
                    save_settings(&self.settings);
                }

                // 2. 編集モード時のみ「設定」と「+ 追加」を表示
                if self.settings.edit_mode {
                    ui.add_space(6.0);

                    // 設定画面へ移動するボタン
                    let settings_btn = egui::Button::new(
                        egui::RichText::new("設定")
                            .color(egui::Color32::from_rgb(74, 85, 104))
                            .size(13.0),
                    )
                    .fill(egui::Color32::from_rgb(237, 242, 247))
                    .corner_radius(4.0);

                    if ui
                        .add_sized(egui::vec2(60.0, 30.0), settings_btn)
                        .on_hover_text("設定画面を開く")
                        .clicked()
                    {
                        self.current_screen = CurrentScreen::Settings;
                    }

                    ui.add_space(6.0);

                    let add_btn = egui::Button::new(
                        egui::RichText::new("+ 追加")
                            .color(egui::Color32::WHITE)
                            .size(13.0),
                    )
                    .fill(egui::Color32::from_rgb(72, 187, 120))
                    .corner_radius(4.0);

                    if ui
                        .add_sized(egui::vec2(80.0, 30.0), add_btn)
                        .on_hover_text("ゲームやアプリを追加 (.exe, .lnk, .url, .html)")
                        .clicked()
                        && let Some(files) = FileDialog::new()
                            .add_filter("ゲーム・アプリ", &["exe", "lnk", "url", "html", "htm"])
                            .pick_files()
                    {
                        self.add_paths(&files);
                    }
                }
            });
        });

        ui.add_space(10.0);

        let sep_stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
        let (sep_rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter()
            .hline(sep_rect.x_range(), sep_rect.center().y, sep_stroke);

        ui.add_space(10.0);

        let mut app_to_launch = None;
        let mut app_to_warn_missing = None;
        let mut app_to_delete = None;
        let mut app_to_rename = None;
        let mut app_to_open_location = None;

        let mut card_rects = Vec::with_capacity(self.apps.len());

        let scroll_output = egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .vertical_scroll_offset(self.scroll_offset)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);
                ui.add_space(6.0);

                for (idx, app) in self.apps.iter().enumerate() {
                    let file_exists = self.file_existence.get(&app.path).copied().unwrap_or(true);
                    let is_launching = self
                        .launching_apps
                        .get(&app.path)
                        .map(|t| t.elapsed() < cooldown)
                        .unwrap_or(false);

                    let is_being_dragged = self.dragging_idx == Some(idx);

                    let card_height = 60.0;
                    let desired_size = egui::vec2(ui.available_width(), card_height);

                    let sense = if self.settings.edit_mode {
                        egui::Sense::click_and_drag()
                    } else {
                        egui::Sense::click()
                    };

                    let (card_rect, card_response) = ui.allocate_exact_size(desired_size, sense);

                    card_rects.push(card_rect);

                    let del_btn_size = egui::vec2(54.0, 28.0);
                    let rename_btn_size = egui::vec2(66.0, 28.0);
                    let location_btn_size = egui::vec2(60.0, 28.0);

                    let del_rect = egui::Rect::from_center_size(
                        egui::pos2(
                            card_rect.right() - 16.0 - del_btn_size.x * 0.5,
                            card_rect.center().y,
                        ),
                        del_btn_size,
                    );
                    let rename_rect = egui::Rect::from_center_size(
                        egui::pos2(
                            del_rect.left() - 6.0 - rename_btn_size.x * 0.5,
                            card_rect.center().y,
                        ),
                        rename_btn_size,
                    );
                    let location_rect = egui::Rect::from_center_size(
                        egui::pos2(
                            rename_rect.left() - 6.0 - location_btn_size.x * 0.5,
                            card_rect.center().y,
                        ),
                        location_btn_size,
                    );

                    let is_hovering_action = self.settings.edit_mode
                        && (ui.rect_contains_pointer(del_rect)
                            || ui.rect_contains_pointer(rename_rect)
                            || ui.rect_contains_pointer(location_rect));

                    if self.settings.edit_mode
                        && !is_hovering_action
                        && card_response.drag_started()
                    {
                        self.dragging_idx = Some(idx);
                    }

                    let is_card_hovered = card_response.hovered() && !is_hovering_action;
                    let is_card_pressed =
                        card_response.is_pointer_button_down_on() && !is_hovering_action;

                    let (bg_color, stroke_color) = if is_being_dragged {
                        (
                            egui::Color32::from_rgb(235, 244, 255),
                            egui::Color32::from_rgb(99, 179, 237),
                        )
                    } else if !file_exists {
                        if is_card_hovered {
                            (
                                egui::Color32::from_rgb(254, 226, 226),
                                egui::Color32::from_rgb(248, 113, 113),
                            )
                        } else {
                            (
                                egui::Color32::from_rgb(254, 242, 242),
                                egui::Color32::from_rgb(252, 165, 165),
                            )
                        }
                    } else if is_launching {
                        (
                            egui::Color32::from_rgb(235, 248, 240),
                            egui::Color32::from_rgb(154, 230, 180),
                        )
                    } else if is_card_pressed {
                        (
                            egui::Color32::from_rgb(226, 232, 240),
                            egui::Color32::from_rgb(203, 213, 225),
                        )
                    } else if is_card_hovered {
                        (
                            egui::Color32::from_rgb(240, 244, 248),
                            egui::Color32::from_rgb(203, 213, 225),
                        )
                    } else {
                        (egui::Color32::WHITE, egui::Color32::from_rgb(226, 232, 240))
                    };

                    ui.painter().rect(
                        card_rect,
                        8.0,
                        bg_color,
                        egui::Stroke::new(1.0, stroke_color),
                        egui::StrokeKind::Inside,
                    );

                    if is_card_hovered && self.dragging_idx.is_none() {
                        if !file_exists {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
                        } else if self.settings.edit_mode {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            card_response
                                .clone()
                                .on_hover_text_at_pointer("ドラッグして並び替え / クリックで起動");
                        } else if !is_launching {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                    }

                    // アイコン描画
                    let icon_size = 34.0;
                    let icon_rect = egui::Rect::from_center_size(
                        egui::pos2(
                            card_rect.left() + 16.0 + icon_size * 0.5,
                            card_rect.center().y,
                        ),
                        egui::vec2(icon_size, icon_size),
                    );

                    if !self.icon_textures.contains_key(&app.path)
                        && !self.requested_icons.contains(&app.path)
                    {
                        self.requested_icons.insert(app.path.clone());
                        let _ = self.icon_req_tx.send(app.path.clone());
                    }

                    let icon_tint = if is_being_dragged {
                        egui::Color32::from_white_alpha(120)
                    } else if !file_exists {
                        egui::Color32::from_white_alpha(100)
                    } else {
                        egui::Color32::WHITE
                    };

                    if let Some(Some(texture)) = self.icon_textures.get(&app.path) {
                        ui.painter().image(
                            texture.id(),
                            icon_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            icon_tint,
                        );
                    } else {
                        ui.painter().rect(
                            icon_rect,
                            6.0,
                            egui::Color32::from_rgb(237, 242, 247),
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(226, 232, 240)),
                            egui::StrokeKind::Inside,
                        );
                        let initial = app
                            .name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string();
                        ui.painter().text(
                            icon_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            initial,
                            egui::FontId::proportional(16.0),
                            egui::Color32::from_rgb(100, 116, 139),
                        );
                    }

                    // テキスト描画
                    let buttons_width = if self.settings.edit_mode { 205.0 } else { 32.0 };
                    let text_start_x = card_rect.left() + 16.0 + icon_size + 14.0;
                    let text_max_w = card_rect.right() - buttons_width - text_start_x;

                    let name_font = egui::FontId::proportional(self.settings.app_name_font_size);
                    let name_pos = egui::pos2(text_start_x, card_rect.top() + 11.0);

                    if !file_exists {
                        ui.painter().text(
                            name_pos,
                            egui::Align2::LEFT_TOP,
                            format!("{}  (ファイルが見つかりません)", app.name),
                            name_font,
                            egui::Color32::from_rgb(220, 38, 38),
                        );
                    } else if is_launching {
                        ui.painter().text(
                            name_pos,
                            egui::Align2::LEFT_TOP,
                            format!("{}  (起動中...)", app.name),
                            name_font,
                            egui::Color32::from_rgb(47, 133, 90),
                        );
                    } else {
                        ui.painter().text(
                            name_pos,
                            egui::Align2::LEFT_TOP,
                            &app.name,
                            name_font,
                            if is_being_dragged {
                                egui::Color32::from_rgb(148, 163, 184)
                            } else {
                                egui::Color32::from_rgb(26, 32, 44)
                            },
                        );
                    }

                    let path_font = egui::FontId::proportional(self.settings.app_path_font_size);
                    let path_pos = egui::pos2(
                        text_start_x,
                        card_rect.top() + 11.0 + self.settings.app_name_font_size + 4.0,
                    );

                    let mut display_path = app.path.clone();
                    let galley = ui.painter().layout_no_wrap(
                        display_path.clone(),
                        path_font.clone(),
                        egui::Color32::WHITE,
                    );
                    if galley.size().x > text_max_w {
                        let ratio = (text_max_w / galley.size().x).clamp(0.0, 1.0);
                        let approx_chars = (display_path.chars().count() as f32 * ratio) as usize;
                        display_path = display_path
                            .chars()
                            .take(approx_chars.saturating_sub(3))
                            .collect::<String>();
                        display_path.push_str("...");
                    }

                    let path_color = if !file_exists {
                        egui::Color32::from_rgb(239, 68, 68)
                    } else if is_being_dragged {
                        egui::Color32::from_rgb(160, 174, 192)
                    } else {
                        egui::Color32::from_rgb(113, 128, 150)
                    };

                    ui.painter().text(
                        path_pos,
                        egui::Align2::LEFT_TOP,
                        display_path,
                        path_font,
                        path_color,
                    );

                    // 編集モード時のみアクションボタン群を表示
                    if self.settings.edit_mode {
                        if custom_button(
                            ui,
                            location_rect,
                            ui.id().with(("location_btn", idx)),
                            "フォルダ",
                            egui::Color32::from_rgb(237, 242, 247),
                            egui::Color32::from_rgb(74, 85, 104),
                            true,
                        )
                        .on_hover_text("ファイルの保存先フォルダを開く")
                        .clicked()
                        {
                            app_to_open_location = Some(app.path.clone());
                        }

                        if custom_button(
                            ui,
                            rename_rect,
                            ui.id().with(("rename_btn", idx)),
                            "名前変更",
                            egui::Color32::from_rgb(66, 153, 225),
                            egui::Color32::WHITE,
                            true,
                        )
                        .on_hover_text("登録名を変更")
                        .clicked()
                        {
                            app_to_rename = Some((idx, app.name.clone()));
                        }

                        if custom_button(
                            ui,
                            del_rect,
                            ui.id().with(("del_btn", idx)),
                            "削除",
                            egui::Color32::from_rgb(245, 101, 101),
                            egui::Color32::WHITE,
                            true,
                        )
                        .on_hover_text("一覧から削除")
                        .clicked()
                        {
                            app_to_delete = Some(idx);
                        }
                    }

                    if card_response.clicked() && !is_hovering_action && self.dragging_idx.is_none()
                    {
                        if !file_exists {
                            app_to_warn_missing = Some((app.name.clone(), app.path.clone()));
                        } else if !is_launching {
                            app_to_launch = Some((app.name.clone(), app.path.clone()));
                        }
                    }
                }

                // ドラッグ中の挿入インジケーター線描画
                if let Some(_) = self.dragging_idx
                    && let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos())
                {
                    let mut target_idx = card_rects.len();
                    for (i, rect) in card_rects.iter().enumerate() {
                        if pointer_pos.y < rect.center().y {
                            target_idx = i;
                            break;
                        }
                    }
                    self.drop_target_idx = Some(target_idx);

                    if !card_rects.is_empty() {
                        let raw_line_y = if target_idx < card_rects.len() {
                            card_rects[target_idx].top() - 5.0
                        } else {
                            card_rects.last().unwrap().bottom() + 5.0
                        };

                        let clip = ui.clip_rect();
                        let line_y = raw_line_y.clamp(clip.top() + 4.0, clip.bottom() - 4.0);

                        let left_x = card_rects[0].left();
                        let right_x = card_rects[0].right();
                        let line_stroke =
                            egui::Stroke::new(3.0, egui::Color32::from_rgb(66, 153, 225));
                        ui.painter().hline(left_x..=right_x, line_y, line_stroke);
                        ui.painter().circle_filled(
                            egui::pos2(left_x, line_y),
                            4.0,
                            egui::Color32::from_rgb(66, 153, 225),
                        );
                        ui.painter().circle_filled(
                            egui::pos2(right_x, line_y),
                            4.0,
                            egui::Color32::from_rgb(66, 153, 225),
                        );
                    }
                }
            });

        self.scroll_offset = scroll_output.state.offset.y;

        // ドラッグ中のエッジスクロール＆プレビュー
        if let Some(drag_idx) = self.dragging_idx {
            ui.ctx().request_repaint();

            if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
                let scroll_rect = scroll_output.inner_rect;
                let edge_margin = 35.0;
                let scroll_speed = 6.0;

                if pointer_pos.y < scroll_rect.top() + edge_margin {
                    self.scroll_offset = (self.scroll_offset - scroll_speed).max(0.0);
                } else if pointer_pos.y > scroll_rect.bottom() - edge_margin {
                    self.scroll_offset += scroll_speed;
                }

                if let Some(drag_app) = self.apps.get(drag_idx) {
                    egui::Area::new(egui::Id::new("drag_preview_area"))
                        .fixed_pos(pointer_pos + egui::vec2(16.0, 12.0))
                        .order(egui::Order::Tooltip)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::new()
                                .fill(egui::Color32::from_rgba_premultiplied(45, 55, 72, 230))
                                .corner_radius(6.0)
                                .inner_margin(egui::Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            egui::RichText::new(&drag_app.name)
                                                .color(egui::Color32::WHITE)
                                                .size(13.0)
                                                .strong(),
                                        );
                                    });
                                });
                        });
                }
            }

            if ui.input(|i| i.pointer.any_released()) {
                if let Some(from) = self.dragging_idx.take()
                    && let Some(to) = self.drop_target_idx.take()
                    && from != to
                    && to != from + 1
                {
                    let item = self.apps.remove(from);
                    let new_to = if to > from { to - 1 } else { to };
                    self.apps.insert(new_to.min(self.apps.len()), item);
                    save_apps(&self.apps);
                }
                self.dragging_idx = None;
                self.drop_target_idx = None;
            }
        }

        if let Some(path) = app_to_open_location {
            self.open_location(&path);
        }
        if let Some((name, path)) = app_to_warn_missing {
            self.toast = Some((
                format!("「{}」のファイルが見つかりません:\n{}", name, path),
                ToastKind::Error,
                Instant::now(),
            ));
        }
        if let Some((name, path)) = app_to_launch {
            self.launch(&name, &path);
        }
        if let Some(idx) = app_to_delete {
            self.delete_app(idx);
        }
        if let Some((idx, current_name)) = app_to_rename {
            self.editing_name = Some((idx, current_name));
        }
    }

    // ----------------------------------------------------
    // 設定画面の描画
    // ----------------------------------------------------
    fn render_settings_screen(&mut self, ui: &mut egui::Ui) {
        let mut settings_changed = false;

        // ヘッダー部
        ui.horizontal(|ui| {
            let back_btn = egui::Button::new(
                egui::RichText::new("← 戻る")
                    .color(egui::Color32::from_rgb(45, 55, 72))
                    .size(13.0),
            )
            .fill(egui::Color32::from_rgb(237, 242, 247))
            .corner_radius(4.0);

            if ui
                .add_sized(egui::vec2(72.0, 30.0), back_btn)
                .on_hover_text("ランチャー一覧画面に戻る (Esc)")
                .clicked()
            {
                self.current_screen = CurrentScreen::Launcher;
            }

            ui.add_space(8.0);

            ui.heading(
                egui::RichText::new("設定")
                    .color(egui::Color32::from_rgb(45, 55, 72))
                    .size(22.0)
                    .strong(),
            );
        });

        // Escキーでも戻れるようにする
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.current_screen = CurrentScreen::Launcher;
        }

        ui.add_space(10.0);

        let sep_stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
        let (sep_rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter()
            .hline(sep_rect.x_range(), sep_rect.center().y, sep_stroke);

        ui.add_space(10.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(0.0, 14.0);

                // --- セクション1: 全般設定 ---
                                ui.label(
                                    egui::RichText::new("全般設定")
                                        .size(16.0)
                                        .strong()
                                        .color(egui::Color32::from_rgb(45, 55, 72)),
                                );

                                let general_frame = egui::Frame::new()
                                    .fill(egui::Color32::WHITE)
                                    .corner_radius(8.0)
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        egui::Color32::from_rgb(226, 232, 240),
                                    ))
                                    .inner_margin(egui::Margin::symmetric(16, 14));

                                general_frame.show(ui, |ui| {
                                    ui.set_width(ui.available_width()); // ← 横幅を統一

                                    if ui
                                        .checkbox(
                                            &mut self.settings.confirm_on_delete,
                                            "アプリの削除時に確認ダイアログを表示する",
                                        )
                                        .changed()
                                    {
                                        settings_changed = true;
                                    }

                                    ui.add_space(10.0);

                                    ui.horizontal(|ui| {
                                        ui.label("連続起動クールダウン:");
                                        if ui
                                            .add_sized(
                                                egui::vec2(80.0, 26.0),
                                                egui::DragValue::new(&mut self.settings.launch_cooldown_secs)
                                                    .range(0..=3600)
                                                    .suffix(" 秒"),
                                            )
                                            .on_hover_text("クリックして直接キーボード入力、またはドラッグで変更")
                                            .changed()
                                        {
                                            settings_changed = true;
                                        }
                                    });
                                    ui.label(
                                        egui::RichText::new(
                                            "※ 同じアプリを誤って連打・多重起動するのを防止する待機時間です。",
                                        )
                                        .size(11.0)
                                        .color(egui::Color32::from_rgb(113, 128, 150)),
                                    );
                                });

                                // --- セクション2: 外観・フォント設定 ---
                                ui.label(
                                    egui::RichText::new("外観・フォントサイズ")
                                        .size(16.0)
                                        .strong()
                                        .color(egui::Color32::from_rgb(45, 55, 72)),
                                );

                                let font_frame = egui::Frame::new()
                                    .fill(egui::Color32::WHITE)
                                    .corner_radius(8.0)
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        egui::Color32::from_rgb(226, 232, 240),
                                    ))
                                    .inner_margin(egui::Margin::symmetric(16, 14));

                                font_frame.show(ui, |ui| {
                                    ui.set_width(ui.available_width()); // ← 横幅を統一

                                    ui.horizontal(|ui| {
                                        ui.label("アプリ名の文字サイズ:");
                                        if ui
                                            .add_sized(
                                                egui::vec2(80.0, 26.0),
                                                egui::DragValue::new(&mut self.settings.app_name_font_size)
                                                    .speed(0.5)
                                                    .range(1.0..=100.0)
                                                    .suffix(" px"),
                                            )
                                            .on_hover_text("クリックして直接キーボード入力、またはドラッグで変更")
                                            .changed()
                                        {
                                            settings_changed = true;
                                        }
                                    });

                                    ui.add_space(6.0);

                                    ui.horizontal(|ui| {
                                        ui.label("ファイルパスの文字サイズ:");
                                        if ui
                                            .add_sized(
                                                egui::vec2(80.0, 26.0),
                                                egui::DragValue::new(&mut self.settings.app_path_font_size)
                                                    .speed(0.5)
                                                    .range(1.0..=100.0)
                                                    .suffix(" px"),
                                            )
                                            .on_hover_text("クリックして直接キーボード入力、またはドラッグで変更")
                                            .changed()
                                        {
                                            settings_changed = true;
                                        }
                                    });

                                    ui.add_space(10.0);
                                    ui.label(
                                        egui::RichText::new("表示サンプルプレビュー:")
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(113, 128, 150)),
                                    );

                                    let preview_frame = egui::Frame::new()
                                        .fill(egui::Color32::from_rgb(247, 250, 252))
                                        .corner_radius(6.0)
                                        .stroke(egui::Stroke::new(
                                            1.0,
                                            egui::Color32::from_rgb(226, 232, 240),
                                        ))
                                        .inner_margin(egui::Margin::symmetric(14, 10));

                                    preview_frame.show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            let (icon_rect, _) =
                                                ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                                            ui.painter().rect(
                                                icon_rect,
                                                6.0,
                                                egui::Color32::from_rgb(237, 242, 247),
                                                egui::Stroke::new(1.0, egui::Color32::from_rgb(226, 232, 240)),
                                                egui::StrokeKind::Inside,
                                            );
                                            ui.painter().text(
                                                icon_rect.center(),
                                                egui::Align2::CENTER_CENTER,
                                                "S",
                                                egui::FontId::proportional(16.0),
                                                egui::Color32::from_rgb(100, 116, 139),
                                            );

                                            ui.add_space(8.0);

                                            ui.vertical(|ui| {
                                                ui.label(
                                                    egui::RichText::new("サンプルアプリケーション")
                                                        .size(self.settings.app_name_font_size)
                                                        .color(egui::Color32::from_rgb(26, 32, 44)),
                                                );
                                                ui.label(
                                                    egui::RichText::new("C:\\Program Files\\SampleApp\\sample.exe")
                                                        .size(self.settings.app_path_font_size)
                                                        .color(egui::Color32::from_rgb(113, 128, 150)),
                                                );
                                            });
                                        });
                                    });
                                });

                                // --- セクション3: 設定・データファイルの管理 ---
                                ui.label(
                                    egui::RichText::new("設定ファイル・データ管理")
                                        .size(16.0)
                                        .strong()
                                        .color(egui::Color32::from_rgb(45, 55, 72)),
                                );

                                let file_frame = egui::Frame::new()
                                    .fill(egui::Color32::WHITE)
                                    .corner_radius(8.0)
                                    .stroke(egui::Stroke::new(
                                        1.0,
                                        egui::Color32::from_rgb(226, 232, 240),
                                    ))
                                    .inner_margin(egui::Margin::symmetric(16, 14));

                                file_frame.show(ui, |ui| {
                                    ui.set_width(ui.available_width()); // ← 横幅を統一

                                    ui.label(
                                        egui::RichText::new("設定やアプリ情報はJSONファイルとして保存されています。直接テキストエディタで確認・編集することも可能です。")
                                            .size(12.0)
                                            .color(egui::Color32::from_rgb(74, 85, 104)),
                                    );

                                    ui.add_space(10.0);

                                    let settings_path = get_data_path("settings.json");
                                    let apps_path = get_data_path("apps.json");
                                    let btn_size = egui::vec2(240.0, 28.0); // ← 幅を統一した縦並びボタン

                                    if ui
                                        .add_sized(btn_size, egui::Button::new("設定ファイルを開く (settings.json)"))
                                        .on_hover_text("既定のテキストエディタで settings.json を開きます")
                                        .clicked()
                                    {
                                        Self::open_file_with_default_app(&settings_path);
                                    }

                                    ui.add_space(6.0);

                                    if ui
                                        .add_sized(btn_size, egui::Button::new("アプリ一覧を開く (apps.json)"))
                                        .on_hover_text("既定のテキストエディタで apps.json を開きます")
                                        .clicked()
                                    {
                                        Self::open_file_with_default_app(&apps_path);
                                    }

                                    ui.add_space(6.0);

                                    if ui
                                        .add_sized(btn_size, egui::Button::new("保存先フォルダを開く"))
                                        .on_hover_text("設定ファイルがあるフォルダーをエクスプローラーで開きます")
                                        .clicked()
                                    {
                                        self.open_location(&settings_path.to_string_lossy());
                                    }

                                    ui.add_space(12.0);
                                    ui.separator();
                                    ui.add_space(8.0);

                                    if ui
                                        .button(
                                            egui::RichText::new("設定を初期値に戻す")
                                                .color(egui::Color32::from_rgb(220, 38, 38)),
                                        )
                                        .on_hover_text("フォントサイズやクールダウンなどを初期状態にリセットします")
                                        .clicked()
                                    {
                                        self.settings = Settings::default();
                                        settings_changed = true;
                                        self.toast = Some((
                                            "設定を初期値にリセットしました".to_string(),
                                            ToastKind::Info,
                                            Instant::now(),
                                        ));
                                    }
                                });

                ui.add_space(10.0);
            });

        if settings_changed {
            save_settings(&self.settings);
        }
    }
}
