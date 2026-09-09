use eframe::egui;
use rfd::FileDialog;
use std::time::Duration;

use crate::app::{CurrentScreen, LauncherApp, ToastKind};
use crate::theme;
use crate::views::widgets::custom_button;

pub fn render_launcher_screen(app: &mut LauncherApp, ui: &mut egui::Ui, cooldown: Duration) {
    // 1. ヘッダー部
    ui.horizontal(|ui| {
        ui.heading(
            egui::RichText::new("ゲームランチャー")
                .color(theme::TEXT_PRIMARY)
                .size(22.0)
                .strong(),
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let (edit_label, edit_bg, edit_fg) = if app.settings.edit_mode {
                ("完了", theme::BTN_BLUE, egui::Color32::WHITE)
            } else {
                ("編集", theme::BTN_GRAY_BG, theme::BTN_GRAY_FG)
            };

            let edit_btn =
                egui::Button::new(egui::RichText::new(edit_label).color(edit_fg).size(13.0))
                    .fill(edit_bg)
                    .corner_radius(4.0);

            if ui
                .add_sized(egui::vec2(68.0, 30.0), edit_btn)
                .on_hover_text(if app.settings.edit_mode {
                    "編集モードを終了して固定"
                } else {
                    "アプリの並び替え・追加・設定を行う"
                })
                .clicked()
            {
                app.settings.edit_mode = !app.settings.edit_mode;
                app.save_settings_internal();
            }

            if app.settings.edit_mode {
                ui.add_space(6.0);

                let settings_btn = egui::Button::new(
                    egui::RichText::new("設定")
                        .color(theme::BTN_GRAY_FG)
                        .size(13.0),
                )
                .fill(theme::BTN_GRAY_BG)
                .corner_radius(4.0);

                if ui
                    .add_sized(egui::vec2(60.0, 30.0), settings_btn)
                    .on_hover_text("設定画面を開く")
                    .clicked()
                {
                    app.current_screen = CurrentScreen::Settings;
                }

                ui.add_space(6.0);

                let add_btn = egui::Button::new(
                    egui::RichText::new("+ 追加")
                        .color(egui::Color32::WHITE)
                        .size(13.0),
                )
                .fill(theme::BTN_GREEN)
                .corner_radius(4.0);

                if ui
                    .add_sized(egui::vec2(80.0, 30.0), add_btn)
                    .on_hover_text("ゲームやアプリを追加 (.exe, .lnk, .url, .html)")
                    .clicked()
                    && let Some(files) = FileDialog::new()
                        .add_filter("ゲーム・アプリ", &["exe", "lnk", "url", "html", "htm"])
                        .pick_files()
                {
                    app.add_paths(&files);
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

    let mut card_rects = Vec::with_capacity(app.apps.len());

    let scroll_output = egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .vertical_scroll_offset(app.scroll_offset)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 10.0);
            ui.add_space(6.0);

            for idx in 0..app.apps.len() {
                let (c_rect, clicked, launch_req, warn_req, del_req, rename_req, open_req) =
                    render_app_card(app, ui, idx, cooldown);

                card_rects.push(c_rect);

                if let Some(r) = launch_req {
                    app_to_launch = Some(r);
                }
                if let Some(r) = warn_req {
                    app_to_warn_missing = Some(r);
                }
                if let Some(r) = del_req {
                    app_to_delete = Some(r);
                }
                if let Some(r) = rename_req {
                    app_to_rename = Some(r);
                }
                if let Some(r) = open_req {
                    app_to_open_location = Some(r);
                }
                let _ = clicked;
            }

            // ドラッグ中の挿入インジケーター線描画
            if app.dragging_idx.is_some()
                && let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos())
            {
                let mut target_idx = card_rects.len();
                for (i, rect) in card_rects.iter().enumerate() {
                    if pointer_pos.y < rect.center().y {
                        target_idx = i;
                        break;
                    }
                }
                app.drop_target_idx = Some(target_idx);

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
                    let line_stroke = egui::Stroke::new(3.0, theme::BTN_BLUE);
                    ui.painter().hline(left_x..=right_x, line_y, line_stroke);
                    ui.painter()
                        .circle_filled(egui::pos2(left_x, line_y), 4.0, theme::BTN_BLUE);
                    ui.painter()
                        .circle_filled(egui::pos2(right_x, line_y), 4.0, theme::BTN_BLUE);
                }
            }
        });

    app.scroll_offset = scroll_output.state.offset.y;

    // ドラッグ中のエッジスクロール＆プレビュー
    if let Some(drag_idx) = app.dragging_idx {
        ui.ctx().request_repaint();

        if let Some(pointer_pos) = ui.input(|i| i.pointer.hover_pos()) {
            let scroll_rect = scroll_output.inner_rect;
            let edge_margin = 35.0;
            let scroll_speed = 6.0;

            if pointer_pos.y < scroll_rect.top() + edge_margin {
                app.scroll_offset = (app.scroll_offset - scroll_speed).max(0.0);
            } else if pointer_pos.y > scroll_rect.bottom() - edge_margin {
                app.scroll_offset += scroll_speed;
            }

            if let Some(drag_app) = app.apps.get(drag_idx) {
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
            if let Some(from) = app.dragging_idx.take()
                && let Some(to) = app.drop_target_idx.take()
                && from != to
                && to != from + 1
            {
                let item = app.apps.remove(from);
                let new_to = if to > from { to - 1 } else { to };
                app.apps.insert(new_to.min(app.apps.len()), item);
                app.save_apps_internal();
            }
            app.dragging_idx = None;
            app.drop_target_idx = None;
        }
    }

    if let Some(path) = app_to_open_location {
        app.open_location(&path);
    }
    if let Some((name, path)) = app_to_warn_missing {
        app.toast = Some((
            format!("「{}」のファイルが見つかりません:\n{}", name, path),
            ToastKind::Error,
            std::time::Instant::now(),
        ));
    }
    if let Some((name, path)) = app_to_launch {
        app.launch(&name, &path);
    }
    if let Some(idx) = app_to_delete {
        app.delete_app(idx);
    }
    if let Some((idx, current_name)) = app_to_rename {
        app.editing_name = Some((idx, current_name));
    }
}

type CardActionResult = (
    egui::Rect,
    bool,
    Option<(String, String)>,
    Option<(String, String)>,
    Option<usize>,
    Option<(usize, String)>,
    Option<String>,
);

/// 単一カードの描画処理を抽出
fn render_app_card(
    app: &mut LauncherApp,
    ui: &mut egui::Ui,
    idx: usize,
    cooldown: Duration,
) -> CardActionResult {
    let target_app = &app.apps[idx];
    let file_exists = app
        .file_existence
        .get(&target_app.path)
        .copied()
        .unwrap_or(true);
    let is_launching = app
        .launching_apps
        .get(&target_app.path)
        .map(|t| t.elapsed() < cooldown)
        .unwrap_or(false);
    let is_being_dragged = app.dragging_idx == Some(idx);

    let card_height = 60.0;
    let desired_size = egui::vec2(ui.available_width(), card_height);
    let sense = if app.settings.edit_mode {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };

    let (card_rect, card_response) = ui.allocate_exact_size(desired_size, sense);

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

    let is_hovering_action = app.settings.edit_mode
        && (ui.rect_contains_pointer(del_rect)
            || ui.rect_contains_pointer(rename_rect)
            || ui.rect_contains_pointer(location_rect));

    if app.settings.edit_mode && !is_hovering_action && card_response.drag_started() {
        app.dragging_idx = Some(idx);
    }

    let is_card_hovered = card_response.hovered() && !is_hovering_action;
    let is_card_pressed = card_response.is_pointer_button_down_on() && !is_hovering_action;

    let (bg_color, stroke_color) = if is_being_dragged {
        (theme::BG_CARD_DRAGGING, theme::BORDER_CARD_DRAGGING)
    } else if !file_exists {
        if is_card_hovered {
            (
                theme::BG_CARD_MISSING_HOVER,
                theme::BORDER_CARD_MISSING_HOVER,
            )
        } else {
            (theme::BG_CARD_MISSING, theme::BORDER_CARD_MISSING)
        }
    } else if is_launching {
        (theme::BG_CARD_LAUNCHING, theme::BORDER_CARD_LAUNCHING)
    } else if is_card_pressed {
        (theme::BG_CARD_PRESSED, theme::BORDER_DEFAULT)
    } else if is_card_hovered {
        (theme::BG_CARD_HOVER, theme::BORDER_DEFAULT)
    } else {
        (theme::BG_CARD, theme::BORDER_DEFAULT)
    };

    ui.painter().rect(
        card_rect,
        8.0,
        bg_color,
        egui::Stroke::new(1.0, stroke_color),
        egui::StrokeKind::Inside,
    );

    if is_card_hovered && app.dragging_idx.is_none() {
        if !file_exists {
            ui.ctx().set_cursor_icon(egui::CursorIcon::NotAllowed);
        } else if app.settings.edit_mode {
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

    if !app.icon_textures.contains_key(&target_app.path)
        && !app.requested_icons.contains(&target_app.path)
    {
        app.requested_icons.insert(target_app.path.clone());
        let _ = app.icon_req_tx.send(target_app.path.clone());
    }

    let icon_tint = if is_being_dragged {
        egui::Color32::from_white_alpha(120)
    } else if !file_exists {
        egui::Color32::from_white_alpha(100)
    } else {
        egui::Color32::WHITE
    };

    if let Some(Some(texture)) = app.icon_textures.get(&target_app.path) {
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
            theme::BTN_GRAY_BG,
            egui::Stroke::new(1.0, theme::BORDER_DEFAULT),
            egui::StrokeKind::Inside,
        );
        let initial = target_app
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
            theme::TEXT_MUTED,
        );
    }

    // テキスト描画
    let buttons_width = if app.settings.edit_mode { 205.0 } else { 32.0 };
    let text_start_x = card_rect.left() + 16.0 + icon_size + 14.0;
    let text_max_w = card_rect.right() - buttons_width - text_start_x;

    let name_font = egui::FontId::proportional(app.settings.app_name_font_size);
    let name_pos = egui::pos2(text_start_x, card_rect.top() + 11.0);

    if !file_exists {
        ui.painter().text(
            name_pos,
            egui::Align2::LEFT_TOP,
            format!("{}  (ファイルが見つかりません)", target_app.name),
            name_font,
            theme::TEXT_ERROR,
        );
    } else if is_launching {
        ui.painter().text(
            name_pos,
            egui::Align2::LEFT_TOP,
            format!("{}  (起動中...)", target_app.name),
            name_font,
            theme::TEXT_SUCCESS,
        );
    } else {
        ui.painter().text(
            name_pos,
            egui::Align2::LEFT_TOP,
            &target_app.name,
            name_font,
            if is_being_dragged {
                egui::Color32::from_rgb(148, 163, 184)
            } else {
                egui::Color32::from_rgb(26, 32, 44)
            },
        );
    }

    let path_font = egui::FontId::proportional(app.settings.app_path_font_size);
    let path_pos = egui::pos2(
        text_start_x,
        card_rect.top() + 11.0 + app.settings.app_name_font_size + 4.0,
    );

    let mut display_path = target_app.path.clone();
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
        theme::TEXT_ERROR
    } else if is_being_dragged {
        egui::Color32::from_rgb(160, 174, 192)
    } else {
        theme::TEXT_MUTED
    };

    ui.painter().text(
        path_pos,
        egui::Align2::LEFT_TOP,
        display_path,
        path_font,
        path_color,
    );

    let mut open_req = None;
    let mut rename_req = None;
    let mut del_req = None;

    if app.settings.edit_mode {
        if custom_button(
            ui,
            location_rect,
            ui.id().with(("location_btn", idx)),
            "フォルダ",
            theme::BTN_GRAY_BG,
            theme::BTN_GRAY_FG,
            true,
        )
        .on_hover_text("ファイルの保存先フォルダを開く")
        .clicked()
        {
            open_req = Some(target_app.path.clone());
        }

        if custom_button(
            ui,
            rename_rect,
            ui.id().with(("rename_btn", idx)),
            "名前変更",
            theme::BTN_BLUE,
            egui::Color32::WHITE,
            true,
        )
        .on_hover_text("登録名を変更")
        .clicked()
        {
            rename_req = Some((idx, target_app.name.clone()));
        }

        if custom_button(
            ui,
            del_rect,
            ui.id().with(("del_btn", idx)),
            "削除",
            theme::BTN_RED,
            egui::Color32::WHITE,
            true,
        )
        .on_hover_text("一覧から削除")
        .clicked()
        {
            del_req = Some(idx);
        }
    }

    let mut launch_req = None;
    let mut warn_req = None;

    if card_response.clicked() && !is_hovering_action && app.dragging_idx.is_none() {
        if !file_exists {
            warn_req = Some((target_app.name.clone(), target_app.path.clone()));
        } else if !is_launching {
            launch_req = Some((target_app.name.clone(), target_app.path.clone()));
        }
    }

    (
        card_rect,
        card_response.clicked(),
        launch_req,
        warn_req,
        del_req,
        rename_req,
        open_req,
    )
}
