// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use chrono::{Local, NaiveDate};
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, TextFormat, Ui,
    UiBuilder, Vec2, pos2, vec2,
};
use zirbe_store::{Conversation, Message};

use crate::dates;
use crate::list;
use crate::palette::Palette;
use crate::system::Clock;

const GAP_FOR_A_TIME: i64 = 60 * 60;
const PADDING: Vec2 = vec2(12.0, 8.0);
const EDGE: f32 = 12.0;
const AVATAR: f32 = 30.0;
const CHIP: f32 = 26.0;
const CHIP_GAP: f32 = 4.0;

pub enum Choice {
    Email { images: bool },
    Open(usize),
    Save(usize),
}

pub struct Action {
    pub message: Message,
    pub choice: Choice,
}

pub fn show(
    ui: &mut Ui,
    conversation: &Conversation,
    me: &str,
    today: NaiveDate,
    clock: Clock,
    palette: &Palette,
) -> Option<Action> {
    let mut action = None;
    let group = conversation.members.len() > 1;
    let widest = (ui.available_width() * 0.72).min(560.0);
    let mut previous: Option<&Message> = None;
    for message in &conversation.messages {
        let mine = message.from.as_ref().is_none_or(|from| from.is(me));
        let new_day = previous.is_none_or(|previous| {
            previous.date.with_timezone(&Local).date_naive()
                != message.date.with_timezone(&Local).date_naive()
        });
        let long_pause = previous
            .is_some_and(|previous| (message.date - previous.date).num_seconds() > GAP_FOR_A_TIME);
        if new_day || long_pause {
            ui.add_space(10.0);
            let label = if new_day {
                format!(
                    "{} {}",
                    dates::day(message.date, today),
                    dates::time(message.date, clock)
                )
            } else {
                dates::time(message.date, clock)
            };
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(label).size(11.5).color(palette.secondary));
            });
            ui.add_space(4.0);
        }
        let same_sender = previous.is_some_and(|previous| {
            previous
                .from
                .as_ref()
                .map(|from| from.address.to_lowercase())
                == message
                    .from
                    .as_ref()
                    .map(|from| from.address.to_lowercase())
        }) && !new_day
            && !long_pause;
        if !mine && group && !same_sender {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(EDGE + AVATAR + 8.0 + PADDING.x);
                let name = message.from.as_ref().map_or("Unknown", |from| from.short());
                ui.label(RichText::new(name).size(11.5).color(palette.secondary));
            });
        }
        ui.add_space(if same_sender { 2.0 } else { 6.0 });
        if let Some(choice) = bubble(
            ui,
            message,
            mine,
            widest,
            palette,
            (!mine).then_some(!same_sender || !group),
        ) {
            action = Some(Action {
                message: message.clone(),
                choice,
            });
        }
        previous = Some(message);
    }
    ui.add_space(12.0);
    action
}

fn content(message: &Message, color: Color32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let body = message.text.trim();
    let mut add = |text: &str, size: f32, color: Color32, italics: bool| {
        if !job.text.is_empty() {
            job.append(
                "\n",
                0.0,
                TextFormat::simple(FontId::proportional(size), color),
            );
        }
        job.append(
            text,
            0.0,
            TextFormat {
                font_id: FontId::proportional(size),
                color,
                italics,
                ..TextFormat::default()
            },
        );
    };
    if !body.is_empty() {
        add(body, 14.0, color, false);
    } else if message.attachments.is_empty() {
        add("No text", 14.0, color.gamma_multiply(0.7), true);
    }
    if message.partial {
        add(
            "Large message: only the first part was downloaded",
            11.5,
            color.gamma_multiply(0.75),
            true,
        );
    }
    job.wrap.max_width = wrap;
    job
}

fn bubble(
    ui: &mut Ui,
    message: &Message,
    mine: bool,
    widest: f32,
    palette: &Palette,
    avatar: Option<bool>,
) -> Option<Choice> {
    let (fill, color) = if mine {
        (palette.accent, palette.on_accent)
    } else {
        (palette.received, palette.text)
    };
    let inner = widest - 2.0 * PADDING.x;
    let job = content(message, color, inner);
    let has_text = !job.text.is_empty();
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job.clone()));
    let text = if has_text { galley.size() } else { Vec2::ZERO };
    let labels: Vec<String> = message
        .attachments
        .iter()
        .map(|name| format!("📎 {name}"))
        .collect();
    let chips_width = labels
        .iter()
        .map(|label| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(label.clone(), FontId::proportional(13.0), color)
                    .size()
                    .x
            }) + 2.0 * PADDING.x
        })
        .fold(0.0, f32::max);
    let chips_height = labels.len() as f32 * (CHIP + CHIP_GAP)
        - if has_text || labels.is_empty() {
            0.0
        } else {
            CHIP_GAP
        };
    let size = vec2(text.x.max(chips_width).min(inner), text.y + chips_height) + 2.0 * PADDING;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size.y), Sense::hover());
    let left = if mine {
        row.right() - EDGE - size.x
    } else {
        row.left() + EDGE + AVATAR + 8.0
    };
    let shape = Rect::from_min_size(pos2(left, row.top()), size);
    ui.painter()
        .rect_filled(shape, CornerRadius::same(16), fill);
    if avatar == Some(true) {
        let spot = Rect::from_min_size(
            pos2(row.left() + EDGE, row.bottom() - AVATAR),
            vec2(AVATAR, AVATAR),
        );
        list::avatar(ui, spot, message.from.as_ref());
    }
    let mut chosen = None;
    let mut area = ui.interact(
        shape,
        ui.scope_id()
            .with(("bubble", message.uid, &message.message_id)),
        Sense::click(),
    );
    let inside = shape.shrink2(PADDING);
    if has_text {
        let text_rect = Rect::from_min_size(inside.min, text).expand2(vec2(1.0, 0.0));
        let label = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(text_rect)
                    .layout(Layout::top_down(Align::Min)),
                |ui| ui.add(egui::Label::new(job).wrap().selectable(true)),
            )
            .inner;
        area = area.union(label);
    }
    let mut top = inside.top() + text.y + if has_text { CHIP_GAP } else { 0.0 };
    for (index, label) in labels.into_iter().enumerate() {
        let spot = Rect::from_min_size(pos2(inside.left(), top), vec2(inside.width(), CHIP));
        let chip = ui
            .put(
                spot,
                egui::Button::new(RichText::new(label).size(13.0).color(color))
                    .fill(color.gamma_multiply(0.12))
                    .corner_radius(10.0)
                    .truncate(),
            )
            .on_hover_text("Click to open, right-click to save");
        if chip.clicked() {
            chosen = Some(Choice::Open(index));
        }
        chip.context_menu(|ui| {
            if ui.button("Open").clicked() {
                chosen = Some(Choice::Open(index));
            }
            if ui.button("Save As…").clicked() {
                chosen = Some(Choice::Save(index));
            }
        });
        top += CHIP + CHIP_GAP;
    }
    if message.html {
        area.context_menu(|ui| {
            if ui.button("Email View").clicked() {
                chosen = Some(Choice::Email { images: false });
            }
            if ui.button("Show Images").clicked() {
                chosen = Some(Choice::Email { images: true });
            }
        });
    }
    chosen
}
