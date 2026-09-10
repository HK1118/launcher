use eframe::egui;

use crate::app::{CurrentScreen, LauncherApp, ToastKind};
use crate::models::{Settings, get_data_path};
use crate::theme;

pub fn render_settings_screen(app: &mut LauncherApp, ui: &mut egui::Ui) {
    let mut settings_changed = false;

    ui.horizontal(|ui| {
        let pad_x = (app.settings.button_font_size * 0.8).max(10.0);
        let pad_y = 6.0;
        ui.spacing_mut().button_padding = egui::vec2(pad_x, pad_y);

        let back_btn = egui::Button::new(
            egui::RichText::new("← 戻る")
                .color(theme::TEXT_PRIMARY)
                .size(app.settings.button_font_size),
        )
        .fill(theme::BTN_GRAY_BG)
        .corner_radius(4.0);

        if ui
            .add(back_btn)
            .on_hover_text("ランチャー一覧画面に戻る (Esc)")
            .clicked()
        {
            app.current_screen = CurrentScreen::Launcher;
        }

        ui.add_space(8.0);

        ui.heading(
            egui::RichText::new("設定")
                .color(theme::TEXT_PRIMARY)
                .size(app.settings.header_font_size)
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
                    .size(app.settings.ui_font_size + 3.0)
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

                let chk = egui::Checkbox::new(
                    &mut app.settings.confirm_on_delete,
                    egui::RichText::new("アプリの削除時に確認ダイアログを表示する")
                        .size(app.settings.ui_font_size),
                );
                if ui.add(chk).changed() {
                    settings_changed = true;
                }

                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("連続起動クールダウン:")
                            .size(app.settings.ui_font_size),
                    );
                    if ui
                        .add(
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
                    .size(app.settings.ui_font_size.max(11.0) - 2.0)
                    .color(theme::TEXT_MUTED),
                );
            });

            // --- セクション2: 外観・フォント設定 ---
            ui.label(
                egui::RichText::new("フォントサイズ設定")
                    .size(app.settings.ui_font_size + 3.0)
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

                egui::Grid::new("font_settings_grid")
                    .num_columns(2)
                    .spacing([16.0, 10.0])
                    .show(ui, |ui| {
                        // アプリ名
                        ui.label(
                            egui::RichText::new("アプリ名の文字サイズ:")
                                .size(app.settings.ui_font_size),
                        );
                        if ui
                            .add(
                                egui::DragValue::new(&mut app.settings.app_name_font_size)
                                    .speed(0.5)
                                    .range(8.0..=50.0)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            settings_changed = true;
                        }
                        ui.end_row();

                        // ファイルパス
                        ui.label(
                            egui::RichText::new("ファイルパスの文字サイズ:")
                                .size(app.settings.ui_font_size),
                        );
                        if ui
                            .add(
                                egui::DragValue::new(&mut app.settings.app_path_font_size)
                                    .speed(0.5)
                                    .range(8.0..=40.0)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            settings_changed = true;
                        }
                        ui.end_row();

                        // ボタン
                        ui.label(
                            egui::RichText::new("ボタンの文字サイズ:")
                                .size(app.settings.ui_font_size),
                        );
                        if ui
                            .add(
                                egui::DragValue::new(&mut app.settings.button_font_size)
                                    .speed(0.5)
                                    .range(8.0..=40.0)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            settings_changed = true;
                        }
                        ui.end_row();

                        // ヘッダー
                        ui.label(
                            egui::RichText::new("ヘッダー見出しの文字サイズ:")
                                .size(app.settings.ui_font_size),
                        );
                        if ui
                            .add(
                                egui::DragValue::new(&mut app.settings.header_font_size)
                                    .speed(0.5)
                                    .range(12.0..=50.0)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            settings_changed = true;
                        }
                        ui.end_row();

                        // 一般UI
                        ui.label(
                            egui::RichText::new("一般テキスト・説明文の文字サイズ:")
                                .size(app.settings.ui_font_size),
                        );
                        if ui
                            .add(
                                egui::DragValue::new(&mut app.settings.ui_font_size)
                                    .speed(0.5)
                                    .range(8.0..=30.0)
                                    .suffix(" px"),
                            )
                            .changed()
                        {
                            settings_changed = true;
                        }
                        ui.end_row();
                    });

                ui.add_space(12.0);
                ui.label(
                    egui::RichText::new("表示サンプルプレビュー:")
                        .size(app.settings.ui_font_size - 1.0)
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
                        let icon_size = (app.settings.app_name_font_size
                            + app.settings.app_path_font_size
                            + 4.0)
                            .clamp(32.0, 48.0);
                        let (icon_rect, _) = ui.allocate_exact_size(
                            egui::vec2(icon_size, icon_size),
                            egui::Sense::hover(),
                        );
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
                            egui::FontId::proportional(icon_size * 0.45),
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

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let _ = ui.add(
                                egui::Button::new(
                                    egui::RichText::new("削除")
                                        .size(app.settings.button_font_size)
                                        .color(egui::Color32::WHITE),
                                )
                                .fill(theme::BTN_RED),
                            );
                        });
                    });
                });
            });

            // --- セクション3: 設定・データファイルの管理 ---
            ui.label(
                egui::RichText::new("設定ファイル・データ管理")
                    .size(app.settings.ui_font_size + 3.0)
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
                    egui::RichText::new("設定やアプリ情報はJSONファイルとして保存されています。")
                        .size(app.settings.ui_font_size)
                        .color(theme::BTN_GRAY_FG),
                );

                ui.add_space(10.0);

                let settings_path = get_data_path("settings.json");
                let apps_path = get_data_path("apps.json");

                let open_settings_btn = egui::Button::new(
                    egui::RichText::new("設定ファイルを開く (settings.json)")
                        .size(app.settings.button_font_size),
                );
                if ui
                    .add(open_settings_btn)
                    .on_hover_text("既定のテキストエディタで settings.json を開きます")
                    .clicked()
                {
                    LauncherApp::open_file_with_default_app(&settings_path);
                }

                ui.add_space(6.0);

                let open_apps_btn = egui::Button::new(
                    egui::RichText::new("アプリ一覧を開く (apps.json)")
                        .size(app.settings.button_font_size),
                );
                if ui
                    .add(open_apps_btn)
                    .on_hover_text("既定のテキストエディタで apps.json を開きます")
                    .clicked()
                {
                    LauncherApp::open_file_with_default_app(&apps_path);
                }

                ui.add_space(6.0);

                let open_dir_btn = egui::Button::new(
                    egui::RichText::new("保存先フォルダを開く").size(app.settings.button_font_size),
                );
                if ui
                    .add(open_dir_btn)
                    .on_hover_text("設定ファイルがあるフォルダーをエクスプローラーで開きます")
                    .clicked()
                {
                    app.open_location(&settings_path.to_string_lossy());
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(8.0);

                let reset_btn = egui::Button::new(
                    egui::RichText::new("設定を初期値に戻す")
                        .size(app.settings.button_font_size)
                        .color(theme::TEXT_ERROR),
                );
                if ui
                    .add(reset_btn)
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
