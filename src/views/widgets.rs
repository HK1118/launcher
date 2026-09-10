use eframe::egui;
use std::time::Duration;

use crate::app::{LauncherApp, ToastKind};
use crate::theme;

pub struct CustomButtonProps<'a> {
    pub text: &'a str,
    pub font_size: f32,
    pub bg_color: egui::Color32,
    pub text_color: egui::Color32,
    pub enabled: bool,
}

pub fn custom_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    id: egui::Id,
    props: CustomButtonProps<'_>,
) -> egui::Response {
    let resp = ui.interact(
        rect,
        id,
        if props.enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );

    let (bg, text_color) = if !props.enabled {
        (
            egui::Color32::from_rgb(247, 250, 252),
            egui::Color32::from_rgb(203, 213, 225),
        )
    } else if resp.is_pointer_button_down_on() {
        (
            egui::Color32::from_rgba_premultiplied(
                (props.bg_color.r() as f32 * 0.85) as u8,
                (props.bg_color.g() as f32 * 0.85) as u8,
                (props.bg_color.b() as f32 * 0.85) as u8,
                255,
            ),
            props.text_color,
        )
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        (
            egui::Color32::from_rgba_premultiplied(
                (props.bg_color.r() as f32 * 1.1).min(255.0) as u8,
                (props.bg_color.g() as f32 * 1.1).min(255.0) as u8,
                (props.bg_color.b() as f32 * 1.1).min(255.0) as u8,
                255,
            ),
            props.text_color,
        )
    } else {
        (props.bg_color, props.text_color)
    };

    ui.painter().rect(
        rect,
        4.0,
        bg,
        egui::Stroke::new(1.0, theme::BORDER_DEFAULT),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        props.text,
        egui::FontId::proportional(props.font_size),
        text_color,
    );

    resp
}

pub fn render_rename_modal(app: &mut LauncherApp, ctx: &egui::Context) {
    let Some((idx, ref mut name_buf)) = app.editing_name else {
        return;
    };

    let mut save_clicked = false;
    let mut close_clicked = false;

    egui::Window::new("名前の変更")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new("新しいアプリケーション名を入力してください:")
                    .size(app.settings.ui_font_size),
            );
            ui.add_space(8.0);

            let text_resp = ui.add(
                egui::TextEdit::singleline(name_buf)
                    .font(egui::FontId::proportional(app.settings.ui_font_size))
                    .desired_width(320.0),
            );
            if text_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                save_clicked = true;
            }

            ui.add_space(14.0);

            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let save_btn = egui::Button::new(
                        egui::RichText::new("保存").size(app.settings.button_font_size),
                    );
                    if ui
                        .add(save_btn)
                        .on_hover_text("変更を保存 (Enter)")
                        .clicked()
                    {
                        save_clicked = true;
                    }

                    ui.add_space(8.0);

                    let cancel_btn = egui::Button::new(
                        egui::RichText::new("キャンセル").size(app.settings.button_font_size),
                    );
                    if ui
                        .add(cancel_btn)
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
        if !new_name.is_empty() && idx < app.apps.len() {
            app.apps[idx].name = new_name;
            app.save_apps_internal();
        }
        app.editing_name = None;
    } else if close_clicked {
        app.editing_name = None;
    }
}

pub fn render_toast(app: &mut LauncherApp, ctx: &egui::Context) {
    let Some((ref msg, ref kind, start_time)) = app.toast else {
        return;
    };

    let timeout = match kind {
        ToastKind::Info => Duration::from_secs(4),
        ToastKind::Error => Duration::from_secs(6),
    };

    if start_time.elapsed() > timeout {
        app.toast = None;
        return;
    }

    let msg_clone = msg.clone();
    let (tag, bg, fg) = match kind {
        ToastKind::Info => ("INFO:", theme::TOAST_INFO_BG, egui::Color32::WHITE),
        ToastKind::Error => ("WARN:", theme::TOAST_WARN_BG, egui::Color32::WHITE),
    };

    egui::Area::new(egui::Id::new("toast_notification"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -25.0))
        .show(ctx, |ui| {
            let frame = egui::Frame::new()
                .fill(bg)
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(16, 10));

            let resp = frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(tag)
                            .color(fg)
                            .size(app.settings.ui_font_size + 2.0)
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(&msg_clone)
                            .color(egui::Color32::WHITE)
                            .size(app.settings.ui_font_size),
                    );
                });
            });

            if resp.response.interact(egui::Sense::click()).clicked() {
                app.toast = None;
            }
        });

    ctx.request_repaint_after(Duration::from_millis(200));
}
