#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use notify::{Event, Watcher};
use rfd::FileDialog;
use serde::{Deserialize, Serialize};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

// ============================================================================
// Windows COM ショートカット (.lnk) 解決
// ============================================================================
type Hresult = i32;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const CLSID_SHELL_LINK: Guid = Guid {
    data1: 0x00021401,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const IID_ISHELL_LINK_W: Guid = Guid {
    data1: 0x000214F9,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

const IID_IPERSIST_FILE: Guid = Guid {
    data1: 0x0000010B,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

// Rust 2024 Edition 準拠 (unsafe extern)
#[link(name = "ole32")]
unsafe extern "system" {
    fn CoInitializeEx(pvReserved: *mut std::ffi::c_void, dwCoInit: u32) -> Hresult;
    fn CoCreateInstance(
        rclsid: *const Guid,
        pUnkOuter: *mut std::ffi::c_void,
        dwClsContext: u32,
        riid: *const Guid,
        ppv: *mut *mut std::ffi::c_void,
    ) -> Hresult;
    fn CoUninitialize();
}

#[repr(C)]
struct IShellLinkWVtbl {
    query_interface: unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *const Guid,
        *mut *mut std::ffi::c_void,
    ) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    get_path: unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *mut u16,
        i32,
        *mut std::ffi::c_void,
        u32,
    ) -> Hresult,
}

#[repr(C)]
struct IPersistFileVtbl {
    query_interface: unsafe extern "system" fn(
        *mut std::ffi::c_void,
        *const Guid,
        *mut *mut std::ffi::c_void,
    ) -> Hresult,
    add_ref: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    release: unsafe extern "system" fn(*mut std::ffi::c_void) -> u32,
    get_class_id: unsafe extern "system" fn(*mut std::ffi::c_void, *mut Guid) -> Hresult,
    is_dirty: unsafe extern "system" fn(*mut std::ffi::c_void) -> Hresult,
    load: unsafe extern "system" fn(*mut std::ffi::c_void, *const u16, u32) -> Hresult,
}

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn resolve_lnk(path: &str) -> Option<(String, String)> {
    unsafe {
        let co_result = CoInitializeEx(ptr::null_mut(), 2);
        if co_result < 0 && co_result != -2147417835 {
            return None;
        }

        let mut shell_link: *mut std::ffi::c_void = ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_SHELL_LINK,
            ptr::null_mut(),
            1,
            &IID_ISHELL_LINK_W,
            &mut shell_link,
        );
        if hr < 0 || shell_link.is_null() {
            if co_result >= 0 {
                CoUninitialize();
            }
            return None;
        }

        let sl_vtbl = *(shell_link as *mut *const IShellLinkWVtbl);
        let mut persist_file: *mut std::ffi::c_void = ptr::null_mut();
        let hr = ((*sl_vtbl).query_interface)(shell_link, &IID_IPERSIST_FILE, &mut persist_file);
        if hr < 0 || persist_file.is_null() {
            ((*sl_vtbl).release)(shell_link);
            if co_result >= 0 {
                CoUninitialize();
            }
            return None;
        }

        let pf_vtbl = *(persist_file as *mut *const IPersistFileVtbl);
        let wide_path = to_wide(path);
        let hr = ((*pf_vtbl).load)(persist_file, wide_path.as_ptr(), 0);
        if hr < 0 {
            ((*pf_vtbl).release)(persist_file);
            ((*sl_vtbl).release)(shell_link);
            if co_result >= 0 {
                CoUninitialize();
            }
            return None;
        }

        let mut buf = [0u16; 1024];
        let hr = ((*sl_vtbl).get_path)(shell_link, buf.as_mut_ptr(), 1024, ptr::null_mut(), 0);
        if hr >= 0 {
            let len = buf.iter().position(|&c| c == 0).unwrap_or(0);
            let target_path = String::from_utf16_lossy(&buf[..len]);
            if !target_path.is_empty() {
                let path_obj = Path::new(&target_path);
                let name = path_obj
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                ((*pf_vtbl).release)(persist_file);
                ((*sl_vtbl).release)(shell_link);
                if co_result >= 0 {
                    CoUninitialize();
                }
                return Some((name, target_path));
            }
        }

        ((*pf_vtbl).release)(persist_file);
        ((*sl_vtbl).release)(shell_link);
        if co_result >= 0 {
            CoUninitialize();
        }
        None
    }
}

// ============================================================================
// 設定・データ構造
// ============================================================================
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
struct SavedApp {
    name: String,
    path: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    confirm_on_delete: bool,
    show_edit_buttons: bool,
    app_name_font_size: f32,
    app_path_font_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            confirm_on_delete: true,
            show_edit_buttons: true,
            app_name_font_size: 15.0,
            app_path_font_size: 12.0,
        }
    }
}

fn get_data_path(file_name: &str) -> PathBuf {
    #[cfg(debug_assertions)]
    {
        PathBuf::from(file_name)
    }
    #[cfg(not(debug_assertions))]
    {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|parent| parent.join(file_name)))
            .unwrap_or_else(|| PathBuf::from(file_name))
    }
}

fn load_apps() -> Vec<SavedApp> {
    let path = get_data_path("apps.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_apps(apps: &[SavedApp]) {
    let path = get_data_path("apps.json");
    if let Ok(json) = serde_json::to_string_pretty(apps) {
        let _ = std::fs::write(path, json);
    }
}

fn load_settings() -> Settings {
    let path = get_data_path("settings.json");
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => {
            let defaults = Settings::default();
            if let Ok(json) = serde_json::to_string_pretty(&defaults) {
                let _ = std::fs::write(path, json);
            }
            defaults
        }
    }
}

// ============================================================================
// 日本語フォント設定
// ============================================================================
fn setup_custom_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let font_candidates = [
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        "C:\\Windows\\Fonts\\YuGothM.ttc",
    ];

    for font_path in font_candidates {
        if let Ok(font_data) = std::fs::read(font_path) {
            fonts.font_data.insert(
                "jp_font".to_owned(),
                Arc::new(egui::FontData::from_owned(font_data)),
            );
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                family.insert(0, "jp_font".to_owned());
            }
            if let Some(family) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                family.insert(0, "jp_font".to_owned());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

// ============================================================================
// egui アプリ本体
// ============================================================================
struct LauncherApp {
    apps: Vec<SavedApp>,
    settings: Settings,
    toast: Option<(String, Instant)>,
    file_receiver: Receiver<()>,
    _watcher: Option<notify::RecommendedWatcher>,
    editing_name: Option<(usize, String)>,
}

impl LauncherApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_custom_fonts(&cc.egui_ctx);

        let (tx, rx) = channel();
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
                let _ = tx.send(());
                egui_ctx.request_repaint();
            }
        })
        .ok();

        if let Some(ref mut w) = watcher {
            let _ = w.watch(&watch_dir, notify::RecursiveMode::NonRecursive);
        }

        Self {
            apps: load_apps(),
            settings: load_settings(),
            toast: None,
            file_receiver: rx,
            _watcher: watcher,
            editing_name: None,
        }
    }

    fn add_paths(&mut self, paths: &[PathBuf]) {
        let allowed = ["exe", "lnk"];
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
            let (name, target_path) = if ext == "lnk" {
                resolve_lnk(&path_str).unwrap_or_else(|| {
                    let n = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    (n, path_str)
                })
            } else {
                let n = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Unknown")
                    .to_string();
                (n, path_str)
            };

            if self.apps.iter().any(|app| app.path == target_path) {
                continue;
            }

            self.apps.push(SavedApp {
                name,
                path: target_path,
            });
            added = true;
        }

        if added {
            save_apps(&self.apps);
        }
    }

    fn launch(&mut self, path_str: &str) {
        let target_path = Path::new(path_str);
        let mut cmd = std::process::Command::new(target_path);

        if let Some(parent) = target_path.parent()
            && parent.exists()
            && parent.is_dir()
        {
            cmd.current_dir(parent);
        }

        if let Err(err) = cmd.spawn() {
            let msg = format!("{} を起動できませんでした: {}", path_str, err);
            self.toast = Some((msg, Instant::now()));
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

        self.apps.remove(idx);
        save_apps(&self.apps);
    }
}

// eframe 0.36 App トレイト
impl eframe::App for LauncherApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // ダークモードを完全に遮断し、ライトテーマを常時適用
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = egui::Color32::from_rgb(247, 250, 252);
        ui.ctx().set_visuals(visuals);

        // ホットリロードの反映
        if self.file_receiver.try_recv().is_ok() {
            let loaded = load_apps();
            if loaded != self.apps {
                self.apps = loaded;
            }
        }

        // ドラッグ＆ドロップ処理
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

        // 背景全面パネル (#f7fafc)
        let panel_frame = egui::Frame::new()
            .fill(egui::Color32::from_rgb(247, 250, 252))
            .inner_margin(egui::Margin::symmetric(20, 16));

        egui::CentralPanel::default()
            .frame(panel_frame)
            .show(ui, |ui| {
                // ヘッダー部
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

                            if ui.add_sized(egui::vec2(80.0, 30.0), add_btn).clicked()
                                && let Some(files) = FileDialog::new()
                                    .add_filter("実行可能ファイル", &["exe", "lnk"])
                                    .pick_files()
                            {
                                self.add_paths(&files);
                            }
                        }
                    });
                });

                ui.add_space(10.0);

                // 黒いセパレーター（区切り線）
                let sep_stroke = egui::Stroke::new(1.0, egui::Color32::BLACK);
                let (sep_rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), 1.0),
                    egui::Sense::hover(),
                );
                ui.painter()
                    .hline(sep_rect.x_range(), sep_rect.center().y, sep_stroke);

                ui.add_space(10.0);

                // アプリ一覧
                let mut app_to_launch = None;
                let mut app_to_delete = None;
                let mut app_to_rename = None;

                egui::ScrollArea::vertical()
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);

                        for (idx, app) in self.apps.iter().enumerate() {
                            let card_height = 60.0;
                            let available_w = ui.available_width();
                            let desired_size = egui::vec2(available_w, card_height);

                            // 1. カード全体の領域を確保
                            let (card_rect, card_response) =
                                ui.allocate_exact_size(desired_size, egui::Sense::click());

                            // 2. ボタン領域の計算 (削除 + 名前変更)
                            let del_btn_size = egui::vec2(60.0, 28.0);
                            let rename_btn_size = egui::vec2(72.0, 28.0);

                            let del_rect = egui::Rect::from_center_size(
                                egui::pos2(
                                    card_rect.right() - 16.0 - del_btn_size.x * 0.5,
                                    card_rect.center().y,
                                ),
                                del_btn_size,
                            );
                            let rename_rect = egui::Rect::from_center_size(
                                egui::pos2(
                                    del_rect.left() - 8.0 - rename_btn_size.x * 0.5,
                                    card_rect.center().y,
                                ),
                                rename_btn_size,
                            );

                            let is_hovering_del = if self.settings.show_edit_buttons {
                                ui.rect_contains_pointer(del_rect)
                            } else {
                                false
                            };
                            let is_hovering_rename = if self.settings.show_edit_buttons {
                                ui.rect_contains_pointer(rename_rect)
                            } else {
                                false
                            };
                            let is_hovering_action = is_hovering_del || is_hovering_rename;

                            // 3. カード本体のホバー・押下判定
                            let is_card_hovered = card_response.hovered() && !is_hovering_action;
                            let is_card_pressed =
                                card_response.is_pointer_button_down_on() && !is_hovering_action;

                            // 4. 背景色・枠線色
                            let (bg_color, stroke_color) = if is_card_pressed {
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
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }

                            // 5. テキスト描画 (Painter直描きのため文字選択カーソルが出ず、全域クリック可能)
                            let buttons_width = if self.settings.show_edit_buttons {
                                170.0
                            } else {
                                32.0
                            };
                            let text_max_w = card_rect.width() - buttons_width;

                            let name_font =
                                egui::FontId::proportional(self.settings.app_name_font_size);
                            let name_pos =
                                egui::pos2(card_rect.left() + 16.0, card_rect.top() + 11.0);
                            ui.painter().text(
                                name_pos,
                                egui::Align2::LEFT_TOP,
                                &app.name,
                                name_font,
                                egui::Color32::from_rgb(26, 32, 44),
                            );

                            let path_font =
                                egui::FontId::proportional(self.settings.app_path_font_size);
                            let path_pos = egui::pos2(
                                card_rect.left() + 16.0,
                                card_rect.top() + 11.0 + self.settings.app_name_font_size + 4.0,
                            );

                            let mut display_path = app.path.clone();
                            let galley = ui.painter().layout_no_wrap(
                                display_path.clone(),
                                path_font.clone(),
                                egui::Color32::WHITE,
                            );
                            if galley.size().x > text_max_w {
                                while display_path.len() > 8 {
                                    display_path.pop();
                                    let g = ui.painter().layout_no_wrap(
                                        format!("{}...", display_path),
                                        path_font.clone(),
                                        egui::Color32::WHITE,
                                    );
                                    if g.size().x <= text_max_w {
                                        display_path = format!("{}...", display_path);
                                        break;
                                    }
                                }
                            }

                            ui.painter().text(
                                path_pos,
                                egui::Align2::LEFT_TOP,
                                display_path,
                                path_font,
                                egui::Color32::from_rgb(113, 128, 150),
                            );

                            // 6. 各種操作ボタン
                            if self.settings.show_edit_buttons {
                                // 名前変更ボタン
                                let rename_resp = ui.interact(
                                    rename_rect,
                                    ui.id().with(("rename_btn", idx)),
                                    egui::Sense::click(),
                                );
                                let rename_bg = if rename_resp.is_pointer_button_down_on() {
                                    egui::Color32::from_rgb(49, 130, 206)
                                } else if rename_resp.hovered() {
                                    egui::Color32::from_rgb(99, 179, 237)
                                } else {
                                    egui::Color32::from_rgb(66, 153, 225)
                                };

                                if rename_resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }

                                ui.painter().rect(
                                    rename_rect,
                                    4.0,
                                    rename_bg,
                                    egui::Stroke::NONE,
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    rename_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "名前変更",
                                    egui::FontId::proportional(12.0),
                                    egui::Color32::WHITE,
                                );

                                if rename_resp.clicked() {
                                    app_to_rename = Some((idx, app.name.clone()));
                                }

                                // 削除ボタン
                                let del_resp = ui.interact(
                                    del_rect,
                                    ui.id().with(("del_btn", idx)),
                                    egui::Sense::click(),
                                );
                                let del_bg = if del_resp.is_pointer_button_down_on() {
                                    egui::Color32::from_rgb(229, 62, 62)
                                } else if del_resp.hovered() {
                                    egui::Color32::from_rgb(252, 129, 129)
                                } else {
                                    egui::Color32::from_rgb(245, 101, 101)
                                };

                                if del_resp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }

                                ui.painter().rect(
                                    del_rect,
                                    4.0,
                                    del_bg,
                                    egui::Stroke::NONE,
                                    egui::StrokeKind::Inside,
                                );
                                ui.painter().text(
                                    del_rect.center(),
                                    egui::Align2::CENTER_CENTER,
                                    "削除",
                                    egui::FontId::proportional(12.0),
                                    egui::Color32::WHITE,
                                );

                                if del_resp.clicked() {
                                    app_to_delete = Some(idx);
                                }
                            }

                            // 7. カードクリックでアプリ起動
                            if card_response.clicked() && !is_hovering_action {
                                app_to_launch = Some(app.path.clone());
                            }
                        }
                    });

                if let Some(path) = app_to_launch {
                    self.launch(&path);
                }
                if let Some(idx) = app_to_delete {
                    self.delete_app(idx);
                }
                if let Some((idx, current_name)) = app_to_rename {
                    self.editing_name = Some((idx, current_name));
                }
            });

        // 名前変更モーダルダイアログ
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
                                .clicked()
                            {
                                save_clicked = true;
                            }
                            ui.add_space(8.0);
                            if ui
                                .add_sized(egui::vec2(70.0, 28.0), egui::Button::new("キャンセル"))
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

        // トースト通知
        if let Some((ref msg, start_time)) = self.toast {
            if start_time.elapsed() > Duration::from_secs(5) {
                self.toast = None;
            } else {
                let msg_clone = msg.clone();
                egui::Area::new(egui::Id::new("toast_notification"))
                    .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -25.0))
                    .show(ui.ctx(), |ui| {
                        let frame = egui::Frame::new()
                            .fill(egui::Color32::from_rgb(45, 55, 72))
                            .corner_radius(8.0)
                            .inner_margin(egui::Margin::symmetric(16, 10));

                        let resp = frame.show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new("⚠")
                                        .color(egui::Color32::from_rgb(246, 224, 94))
                                        .size(15.0),
                                );
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

// ============================================================================
// エントリーポイント
// ============================================================================
fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([550.0, 450.0])
            .with_min_inner_size([400.0, 300.0])
            .with_title("Game Launcher"),
        ..Default::default()
    };

    eframe::run_native(
        "Game Launcher",
        options,
        Box::new(|cc| Ok(Box::new(LauncherApp::new(cc)))),
    )
}
