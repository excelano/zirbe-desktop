// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeSet, HashMap};

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
const CONTROLS: f32 = 20.0;
const CONTROL_GAP: f32 = 12.0;
pub const EMAIL_VIEW: &str = "🌐 Email View";
pub const SHOW_IMAGES: &str = "👁 Show Images";

pub enum Choice {
    Email { images: bool },
    Open(usize),
    Save(usize),
    ReplyTo,
    Forward,
    Copy,
    Quoted,
}

pub fn key_of(message: &Message) -> String {
    message
        .message_id
        .clone()
        .unwrap_or_else(|| format!("{}:{}", message.mailbox, message.uid))
}

pub struct View<'a> {
    pub images_by_default: bool,
    pub expanded: &'a HashMap<String, String>,
    pub found: &'a [usize],
    pub current: Option<usize>,
    pub scroll: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Plain,
    Found,
    Current { scroll: bool },
}

struct Style<'a> {
    palette: &'a Palette,
    images_by_default: bool,
    expanded: &'a HashMap<String, String>,
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
    view: &View<'_>,
) -> Option<Action> {
    let style = Style {
        palette,
        images_by_default: view.images_by_default,
        expanded: view.expanded,
    };
    let mut action = None;
    let group = conversation.members.len() > 1;
    let widest = (ui.available_width() * 0.72).min(560.0);
    let mut previous: Option<&Message> = None;
    let mut people_before: Option<BTreeSet<String>> = None;
    for (index, message) in conversation.messages.iter().enumerate() {
        let mark = if view.current == Some(index) {
            Mark::Current {
                scroll: view.scroll,
            }
        } else if view.found.contains(&index) {
            Mark::Found
        } else {
            Mark::Plain
        };
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
        let present = people_of(message, me);
        if let Some(before) = &people_before {
            let names = |addresses: Vec<&String>| -> String {
                addresses
                    .into_iter()
                    .map(|address| {
                        conversation
                            .members
                            .iter()
                            .find(|member| member.is(address))
                            .map_or(address.as_str(), |member| member.short())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let joined: Vec<&String> = present.difference(before).collect();
            let left: Vec<&String> = before.difference(&present).collect();
            for (who, what) in [(joined, "joined"), (left, "left")] {
                if !who.is_empty() {
                    ui.add_space(4.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(format!("{} {what}", names(who)))
                                .size(11.5)
                                .italics()
                                .color(palette.secondary),
                        );
                    });
                }
            }
        }
        people_before = Some(present);
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
            &style,
            (!mine).then_some(!same_sender || !group),
            mark,
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

fn people_of(message: &Message, me: &str) -> BTreeSet<String> {
    message
        .from
        .iter()
        .chain(&message.to)
        .chain(&message.cc)
        .filter(|person| !person.is(me))
        .map(|person| person.address.to_lowercase())
        .collect()
}

fn content(message: &Message, full: Option<&String>, color: Color32, wrap: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    let body = full.unwrap_or(&message.text).trim();
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

pub fn attachment_button(name: &str, color: Color32) -> egui::Button<'static> {
    egui::Button::new(RichText::new(format!("📎 {name}")).size(13.0).color(color))
        .fill(color.gamma_multiply(0.12))
        .corner_radius(10.0)
        .truncate()
}

pub fn attachment_choice(chip: &egui::Response, index: usize) -> Option<Choice> {
    let chip = chip
        .clone()
        .on_hover_text("Click to open, right-click to save");
    let mut chosen = chip.clicked().then_some(Choice::Open(index));
    chip.context_menu(|ui| {
        if ui.button("Open").clicked() {
            chosen = Some(Choice::Open(index));
        }
        if ui.button("Save As…").clicked() {
            chosen = Some(Choice::Save(index));
        }
    });
    chosen
}

fn label_width(ui: &Ui, text: &str, color: Color32) -> f32 {
    ui.fonts_mut(|fonts| {
        fonts
            .layout_no_wrap(text.to_string(), FontId::proportional(13.0), color)
            .size()
            .x
    })
}

fn bubble(
    ui: &mut Ui,
    message: &Message,
    mine: bool,
    widest: f32,
    style: &Style<'_>,
    avatar: Option<bool>,
    mark: Mark,
) -> Option<Choice> {
    let palette = style.palette;
    let images_by_default = style.images_by_default;
    let (fill, color) = if mine {
        (palette.accent, palette.on_accent)
    } else {
        (palette.received, palette.text)
    };
    let tone = if mine {
        palette.on_accent.gamma_multiply(0.85)
    } else {
        palette.secondary
    };
    let inner = widest - 2.0 * PADDING.x;
    let full = style.expanded.get(&key_of(message));
    let job = content(message, full, color, inner);
    let has_text = !job.text.is_empty();
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job.clone()));
    let text = if has_text { galley.size() } else { Vec2::ZERO };
    let separate_images = !images_by_default;
    let controls_width = if !message.html {
        0.0
    } else if separate_images {
        label_width(ui, EMAIL_VIEW, tone) + label_width(ui, SHOW_IMAGES, tone) + 3.0 * CONTROL_GAP
    } else {
        label_width(ui, EMAIL_VIEW, tone)
    };
    let controls_height = if message.html {
        CONTROLS + CHIP_GAP
    } else {
        0.0
    };
    let chips_width = message
        .attachments
        .iter()
        .map(|name| label_width(ui, &format!("📎 {name}"), color) + 2.0 * PADDING.x)
        .fold(0.0, f32::max);
    let chips_height = message.attachments.len() as f32 * (CHIP + CHIP_GAP)
        - if has_text || message.attachments.is_empty() {
            0.0
        } else {
            CHIP_GAP
        };
    let size = vec2(
        text.x.max(chips_width).max(controls_width).min(inner),
        controls_height + text.y + chips_height,
    ) + 2.0 * PADDING;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), size.y), Sense::hover());
    let left = if mine {
        row.right() - EDGE - size.x
    } else {
        row.left() + EDGE + AVATAR + 8.0
    };
    let shape = Rect::from_min_size(pos2(left, row.top()), size);
    ui.painter()
        .rect_filled(shape, CornerRadius::same(16), fill);
    let outline = match mark {
        Mark::Plain => None,
        Mark::Found => Some(egui::Stroke::new(1.5, palette.flag.gamma_multiply(0.6))),
        Mark::Current { scroll } => {
            if scroll {
                ui.scroll_to_rect(shape, Some(Align::Center));
            }
            Some(egui::Stroke::new(2.5, palette.flag))
        }
    };
    if let Some(stroke) = outline {
        ui.painter().rect_stroke(
            shape.expand(2.0),
            CornerRadius::same(18),
            stroke,
            egui::StrokeKind::Outside,
        );
    }
    if avatar == Some(true) {
        let spot = Rect::from_min_size(
            pos2(row.left() + EDGE, row.bottom() - AVATAR),
            vec2(AVATAR, AVATAR),
        );
        list::avatar(ui, spot, message.from.as_ref());
    }
    let mut chosen = None;
    let inside = shape.shrink2(PADDING);
    let mut top = inside.top();
    if message.html {
        let control =
            |text: &str| egui::Button::new(RichText::new(text).size(13.0).color(tone)).frame(false);
        let email_width = label_width(ui, EMAIL_VIEW, tone);
        let images_width = label_width(ui, SHOW_IMAGES, tone);
        let email = Rect::from_min_size(pos2(inside.left(), top), vec2(email_width, CONTROLS));
        let images = Rect::from_min_size(
            pos2(inside.right() - images_width, top),
            vec2(images_width, CONTROLS),
        );
        let hint = if separate_images {
            "Show this message as the sender laid it out, without loading pictures from the sender's servers"
        } else {
            "Show this message as the sender laid it out, with its pictures"
        };
        if ui
            .put(email, control(EMAIL_VIEW))
            .on_hover_text(hint)
            .clicked()
        {
            chosen = Some(Choice::Email {
                images: images_by_default,
            });
        }
        if separate_images {
            let middle = (email.right() + images.left()) / 2.0;
            ui.painter().vline(
                middle,
                (top + 3.0)..=(top + CONTROLS - 3.0),
                egui::Stroke::new(1.0, tone.gamma_multiply(0.5)),
            );
            if ui
                .put(images, control(SHOW_IMAGES))
                .on_hover_text("Show this message as laid out, loading its pictures from the sender's servers, which can tell the sender you opened it")
                .clicked()
            {
                chosen = Some(Choice::Email { images: true });
            }
        }
        top += CONTROLS + CHIP_GAP;
    }
    let mut area = ui.interact(
        shape,
        ui.scope_id()
            .with(("bubble", message.uid, &message.message_id)),
        Sense::click(),
    );
    if has_text {
        let text_rect = Rect::from_min_size(pos2(inside.left(), top), text).expand2(vec2(1.0, 0.0));
        let label = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(text_rect)
                    .layout(Layout::top_down(Align::Min)),
                |ui| ui.add(egui::Label::new(job).wrap().selectable(true)),
            )
            .inner;
        area = area.union(label);
        top += text.y + CHIP_GAP;
    }
    area.context_menu(|ui| {
        let mut item = |ui: &mut Ui, label: &str, choice: Choice| {
            if ui.button(label).clicked() {
                chosen = Some(choice);
                ui.close();
            }
        };
        item(ui, "Reply to This Message", Choice::ReplyTo);
        item(ui, "Forward…", Choice::Forward);
        item(ui, "Copy Text", Choice::Copy);
        let quoted = if full.is_some() {
            "Hide Quoted Text"
        } else {
            "Show Quoted Text"
        };
        item(ui, quoted, Choice::Quoted);
    });
    for (index, name) in message.attachments.iter().enumerate() {
        let spot = Rect::from_min_size(pos2(inside.left(), top), vec2(inside.width(), CHIP));
        let chip = ui.put(spot, attachment_button(name, color));
        if let Some(choice) = attachment_choice(&chip, index) {
            chosen = Some(choice);
        }
        top += CHIP + CHIP_GAP;
    }
    chosen
}
