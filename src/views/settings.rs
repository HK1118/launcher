use eframe::egui;

use crate::app::{CurrentScreen, LauncherApp, ToastKind};
use crate::models::{Settings, get_data_path};
use crate::theme;

pub fn render_settings_screen(app: &mut LauncherApp, ui: &mut egui::Ui) {
    let mut settings_changed = false;

    ui.horizontal(|ui| {
        let back_btn = egui::Button::new(
            egui::RichText::new("← 戻る")
                .color(theme::TEXT_PRIMARY)
                .size(13.0),
        )
        .fill(theme::BTN_GRAY_BG)
        .corner_radius(4.0);

        if ui
            .add_sized(egui::vec2(72.0, 30.0), back_btn)
            .on_hover_text("ランチャー一覧画面に戻る (Esc)")
            .clicked()
        {
            app.current_screen = CurrentScreen::Launcher;
        }

        ui.add_space(8.0);

        ui.heading(
            egui::RichText::new("設定")
                .color(theme::TEXT_PRIMARY)
                .size(22.0)
                .strong(),
        );
    });

    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.current_screen = CurrentScreen::Launcher;
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
                    .color(theme::TEXT_PRIMARY),
            );

            let general_frame = egui::Frame::new()
                .fill(egui::Color32::WHITE)
                .corner_radius(8.0)
                .stroke(egui::Stroke::new(1.0, theme::BORDER_DEFAULT))
                .inner_margin(egui::Margin::symmetric(16, 14));

            general_frame.show(ui, |ui| {
                ui.set_width(ui.available_width());

                if ui
                    .checkbox(
                        &mut app.settings.confirm_on_delete,
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
                            egui::DragValue::new(&mut app.settings.launch_cooldown_secs)
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
                    .color(theme::TEXT_MUTED),
                );
            });

            // --- セクション2: 外観・フォント設定 ---
            ui.label(
                egui::RichText::new("外観・フォントサイズ")
                    .size(16.0)
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );

            let font_frame = egui::Frame::new()
                .fill(egui::Color32::WHITE)
                .corner_radius(8.0)
                .stroke(egui::Stroke::new(1.0, theme::BORDER_DEFAULT))
                .inner_margin(egui::Margin::symmetric(16, 14));

            font_frame.show(ui, |ui| {
                ui.set_width(ui.available_width());

                ui.horizontal(|ui| {
                    ui.label("アプリ名の文字サイズ:");
                    if ui
                        .add_sized(
                            egui::vec2(80.0, 26.0),
                            egui::DragValue::new(&mut app.settings.app_name_font_size)
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
                            egui::DragValue::new(&mut app.settings.app_path_font_size)
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
                        .color(theme::TEXT_MUTED),
                );

                let preview_frame = egui::Frame::new()
                    .fill(theme::BG_APP)
                    .corner_radius(6.0)
                    .stroke(egui::Stroke::new(1.0, theme::BORDER_DEFAULT))
                    .inner_margin(egui::Margin::symmetric(14, 10));

                preview_frame.show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (icon_rect, _) =
                            ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                        ui.painter().rect(
                            icon_rect,
                            6.0,
                            theme::BTN_GRAY_BG,
                            egui::Stroke::new(1.0, theme::BORDER_DEFAULT),
                            egui::StrokeKind::Inside,
                        );
                        ui.painter().text(
                            icon_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "S",
                            egui::FontId::proportional(16.0),
                            theme::TEXT_MUTED,
                        );

                        ui.add_space(8.0);

                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new("サンプルアプリケーション")
                                    .size(app.settings.app_name_font_size)
                                    .color(egui::Color32::from_rgb(26, 32, 44)),
                            );
                            ui.label(
                                egui::RichText::new("C:\\Program Files\\SampleApp\\sample.exe")
                                    .size(app.settings.app_path_font_size)
                                    .color(theme::TEXT_MUTED),
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
                    .color(theme::TEXT_PRIMARY),
            );

            let file_frame = egui::Frame::new()
                .fill(egui::Color32::WHITE)
                .corner_radius(8.0)
                .stroke(egui::Stroke::new(1.0, theme::BORDER_DEFAULT))
                .inner_margin(egui::Margin::symmetric(16, 14));

            file_frame.show(ui, |ui| {
                ui.set_width(ui.available_width());

                ui.label(
                    egui::RichText::new("設定やアプリ情報はJSONファイルとして保存されています。直接テキストエディタで確認・編集することも可能です。")
                        .size(12.0)
                        .color(theme::BTN_GRAY_FG),
                );

                ui.add_space(10.0);

                let settings_path = get_data_path("settings.json");
                let apps_path = get_data_path("apps.json");
                let btn_size = egui::vec2(240.0, 28.0);

                if ui
                    .add_sized(btn_size, egui::Button::new("設定ファイルを開く (settings.json)"))
                    .on_hover_text("既定のテキストエディタで settings.json を開きます")
                    .clicked()
                {
                    LauncherApp::open_file_with_default_app(&settings_path);
                }

                ui.add_space(6.0);

                if ui
                    .add_sized(btn_size, egui::Button::new("アプリ一覧を開く (apps.json)"))
                    .on_hover_text("既定のテキストエディタで apps.json を開きます")
                    .clicked()
                {
                    LauncherApp::open_file_with_default_app(&apps_path);
                }

                ui.add_space(6.0);

                if ui
                    .add_sized(btn_size, egui::Button::new("保存先フォルダを開く"))
                    .on_hover_text("設定ファイルがあるフォルダーをエクスプローラーで開きます")
                    .clicked()
                {
                    app.open_location(&settings_path.to_string_lossy());
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                if ui
                    .button(
                        egui::RichText::new("設定を初期値に戻す")
                            .color(theme::TEXT_ERROR),
                    )
                    .on_hover_text("フォントサイズやクールダウンなどを初期状態にリセットします")
                    .clicked()
                {
                    app.settings = Settings::default();
                    settings_changed = true;
                    app.toast = Some((
                        "設定を初期値にリセットしました".to_string(),
                        ToastKind::Info,
                        std::time::Instant::now(),
                    ));
                }
            });

            ui.add_space(10.0);
        });

    if settings_changed {
        app.save_settings_internal();
    }
}
