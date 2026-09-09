use eframe::egui;
use notify::{Event, Watcher};
use rfd::FileDialog;
use std::collections::{HashMap, HashSet};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use crate::icon::extract_icon_image;
use crate::models::{SavedApp, Settings, get_data_path, load_apps, load_settings, save_apps};

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

pub struct LauncherApp {
    apps: Vec<SavedApp>,
    settings: Settings,
    toast: Option<(String, ToastKind, Instant)>,
    file_receiver: Receiver<()>,
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
            if let Ok(event) = res
                && event.paths.iter().any(|p| p.ends_with("apps.json"))
            {
                let _ = file_tx.send(());
                egui_ctx.request_repaint();
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

        // アイコン抽出のバックグラウンドワーカー（COM初期化を追加）
        let (req_tx, req_rx) = channel::<String>();
        let (res_tx, res_rx) = channel::<(String, Option<egui::ColorImage>)>();
        let bg_ctx = cc.egui_ctx.clone();

        std::thread::spawn(move || {
            unsafe {
                // 2 = COINIT_APARTMENTTHREADED (Shell API とショートカット解決に必須)
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
        save_apps(&self.apps);
    }

    fn open_location(&mut self, path_str: &str) {
        let target = Path::new(path_str);
        if target.exists() {
            if let Err(e) = std::process::Command::new("explorer")
                .raw_arg(format!("/select,\"{}\"", target.to_string_lossy()))
                .spawn()
            {
                self.toast = Some((
                    format!("エクスプローラーの起動に失敗しました: {}", e),
                    ToastKind::Error,
                    Instant::now(),
                ));
            }
        } else if let Some(parent) = target.parent()
            && parent.exists()
        {
            if let Err(e) = std::process::Command::new("explorer").arg(parent).spawn() {
                self.toast = Some((
                    format!("エクスプローラーの起動に失敗しました: {}", e),
                    ToastKind::Error,
                    Instant::now(),
                ));
            }
        } else {
            self.toast = Some((
                format!("保存場所が見つかりません:\n{}", path_str),
                ToastKind::Error,
                Instant::now(),
            ));
        }
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

        if self.file_receiver.try_recv().is_ok()
            && let Ok(loaded) = load_apps()
            && loaded != self.apps
        {
            self.apps = loaded;
            self.refresh_file_existence();
        }

        let is_focused = ui.input(|i| i.focused);
        if (is_focused && !self.last_window_focused)
            || self.last_existence_check.elapsed() > Duration::from_secs(5)
        {
            self.refresh_file_existence();
        }
        self.last_window_focused = is_focused;

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

        let panel_frame = egui::Frame::new()
            .fill(egui::Color32::from_rgb(247, 250, 252))
            .inner_margin(egui::Margin::symmetric(20, 16));

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        egui::RichText::new("ゲームランチャー")
                            .color(egui::Color32::from_rgb(45, 55, 72))
                            .size(22.0)
                            .strong(),
                    );

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.settings.show_edit_buttons {
                            let add_btn = egui::Button::new(
                                egui::RichText::new("+ 追加")
                                    .color(egui::Color32::WHITE)
                                    .size(13.0),
                            )
                            .fill(egui::Color32::from_rgb(72, 187, 120))
                            .corner_radius(4.0);

                            if ui
                                .add_sized(egui::vec2(80.0, 30.0), add_btn)
                                .on_hover_text("ゲームやアプリを追加 (exe, lnk, url, html)")
                                .clicked()
                                && let Some(files) = FileDialog::new()
                                    .add_filter(
                                        "ゲーム・アプリ",
                                        &["exe", "lnk", "url", "html", "htm"],
                                    )
                                    .pick_files()
                            {
                                self.add_paths(&files);
                            }
                        }
                    });
                });

                ui.add_space(10.0);

                let sep_stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
                let (sep_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .hline(sep_rect.x_range(), sep_rect.center().y, sep_stroke);

                ui.add_space(10.0);

                let mut app_to_launch = None;
                let mut app_to_warn_missing = None;
                let mut app_to_delete = None;
                let mut app_to_rename = None;
                let mut app_to_open_location = None;
                let mut app_to_move_up = None;
                let mut app_to_move_down = None;

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);

                        for (idx, app) in self.apps.iter().enumerate() {
                            let file_exists =
                                self.file_existence.get(&app.path).copied().unwrap_or(true);
                            let is_launching = self
                                .launching_apps
                                .get(&app.path)
                                .map(|t| t.elapsed() < cooldown)
                                .unwrap_or(false);

                            let card_height = 60.0;
                            let desired_size = egui::vec2(ui.available_width(), card_height);
                            let (card_rect, card_response) =
                                ui.allocate_exact_size(desired_size, egui::Sense::click());

                            let del_btn_size = egui::vec2(54.0, 28.0);
                            let rename_btn_size = egui::vec2(66.0, 28.0);
                            let location_btn_size = egui::vec2(48.0, 28.0);
                            let arrow_btn_size = egui::vec2(28.0, 28.0);

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
                            let down_rect = egui::Rect::from_center_size(
                                egui::pos2(
                                    location_rect.left() - 6.0 - arrow_btn_size.x * 0.5,
                                    card_rect.center().y,
                                ),
                                arrow_btn_size,
                            );
                            let up_rect = egui::Rect::from_center_size(
                                egui::pos2(
                                    down_rect.left() - 4.0 - arrow_btn_size.x * 0.5,
                                    card_rect.center().y,
                                ),
                                arrow_btn_size,
                            );

                            let is_hovering_action = self.settings.show_edit_buttons
                                && (ui.rect_contains_pointer(del_rect)
                                    || ui.rect_contains_pointer(rename_rect)
                                    || ui.rect_contains_pointer(location_rect)
                                    || ui.rect_contains_pointer(down_rect)
                                    || ui.rect_contains_pointer(up_rect));

                            let is_card_hovered = card_response.hovered() && !is_hovering_action;
                            let is_card_pressed =
                                card_response.is_pointer_button_down_on() && !is_hovering_action;

                            let (bg_color, stroke_color) = if !file_exists {
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

                            if is_card_hovered {
                                if !file_exists {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
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

                            let icon_tint = if !file_exists {
                                egui::Color32::from_white_alpha(100)
                            } else {
                                egui::Color32::WHITE
                            };

                            if let Some(Some(texture)) = self.icon_textures.get(&app.path) {
                                ui.painter().image(
                                    texture.id(),
                                    icon_rect,
                                    egui::Rect::from_min_max(
                                        egui::pos2(0.0, 0.0),
                                        egui::pos2(1.0, 1.0),
                                    ),
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
                                ui.painter().text(
                                    icon_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "🎮",
                                    egui::FontId::proportional(18.0),
                                    egui::Color32::from_rgb(160, 174, 192),
                                );
                            }

                            // テキスト描画
                            let buttons_width = if self.settings.show_edit_buttons {
                                275.0
                            } else {
                                32.0
                            };
                            let text_start_x = card_rect.left() + 16.0 + icon_size + 14.0;
                            let text_max_w = card_rect.right() - buttons_width - text_start_x;

                            let name_font =
                                egui::FontId::proportional(self.settings.app_name_font_size);
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
                                    format!("{}  (起動処理中...)", app.name),
                                    name_font,
                                    egui::Color32::from_rgb(47, 133, 90),
                                );
                            } else {
                                ui.painter().text(
                                    name_pos,
                                    egui::Align2::LEFT_TOP,
                                    &app.name,
                                    name_font,
                                    egui::Color32::from_rgb(26, 32, 44),
                                );
                            }

                            let path_font =
                                egui::FontId::proportional(self.settings.app_path_font_size);
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
                                let approx_chars =
                                    (display_path.chars().count() as f32 * ratio) as usize;
                                display_path = display_path
                                    .chars()
                                    .take(approx_chars.saturating_sub(3))
                                    .collect::<String>();
                                display_path.push_str("...");
                            }

                            let path_color = if !file_exists {
                                egui::Color32::from_rgb(239, 68, 68)
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

                            if self.settings.show_edit_buttons {
                                let can_move_up = idx > 0;
                                if custom_button(
                                    ui,
                                    up_rect,
                                    ui.id().with(("up_btn", idx)),
                                    "▲",
                                    egui::Color32::from_rgb(237, 242, 247),
                                    egui::Color32::from_rgb(74, 85, 104),
                                    can_move_up,
                                )
                                .on_hover_text(if can_move_up {
                                    "上へ移動"
                                } else {
                                    "先頭です"
                                })
                                .clicked()
                                    && can_move_up
                                {
                                    app_to_move_up = Some(idx);
                                }

                                let can_move_down = idx + 1 < self.apps.len();
                                if custom_button(
                                    ui,
                                    down_rect,
                                    ui.id().with(("down_btn", idx)),
                                    "▼",
                                    egui::Color32::from_rgb(237, 242, 247),
                                    egui::Color32::from_rgb(74, 85, 104),
                                    can_move_down,
                                )
                                .on_hover_text(if can_move_down {
                                    "下へ移動"
                                } else {
                                    "末尾です"
                                })
                                .clicked()
                                    && can_move_down
                                {
                                    app_to_move_down = Some(idx);
                                }

                                if custom_button(
                                    ui,
                                    location_rect,
                                    ui.id().with(("location_btn", idx)),
                                    "場所",
                                    egui::Color32::from_rgb(237, 242, 247),
                                    egui::Color32::from_rgb(74, 85, 104),
                                    true,
                                )
                                .on_hover_text("ファイルの保存場所を開く")
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

                            if card_response.clicked() && !is_hovering_action {
                                if !file_exists {
                                    app_to_warn_missing =
                                        Some((app.name.clone(), app.path.clone()));
                                } else if !is_launching {
                                    app_to_launch = Some((app.name.clone(), app.path.clone()));
                                }
                            }
                        }
                    });

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
                if let Some(idx) = app_to_move_up
                    && idx > 0
                    && idx < self.apps.len()
                {
                    self.apps.swap(idx, idx - 1);
                    save_apps(&self.apps);
                }
                if let Some(idx) = app_to_move_down
                    && idx + 1 < self.apps.len()
                {
                    self.apps.swap(idx, idx + 1);
                    save_apps(&self.apps);
                }
            });

        if self.launching_apps.values().any(|t| t.elapsed() < cooldown) {
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }

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

        if let Some((ref msg, ref kind, start_time)) = self.toast {
            let timeout = match kind {
                ToastKind::Info => Duration::from_secs(4),
                ToastKind::Error => Duration::from_secs(6),
            };

            if start_time.elapsed() > timeout {
                self.toast = None;
            } else {
                let msg_clone = msg.clone();
                let (icon, bg, fg) = match kind {
                    ToastKind::Info => (
                        "🚀",
                        egui::Color32::from_rgb(44, 122, 123),
                        egui::Color32::WHITE,
                    ),
                    ToastKind::Error => (
                        "⚠",
                        egui::Color32::from_rgb(45, 55, 72),
                        egui::Color32::from_rgb(246, 224, 94),
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
                                ui.label(egui::RichText::new(icon).color(fg).size(15.0));
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
