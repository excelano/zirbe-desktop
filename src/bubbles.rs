// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use chrono::{Local, NaiveDate};
use eframe::egui::text::LayoutJob;
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, TextFormat, Ui,
    UiBuilder, Vec2, pos2, vec2,
};
use zirbe_store::{Account, Conversation, Message, When};

use crate::audio::{self, Sound, is_audio};
use crate::contacts::Faces;
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
pub const WEB_VIEW: &str = "🌐 Web View";
pub const SHOW_IMAGES: &str = "👁 Show Images";
const SHOW_QUOTED: &str = "Show quoted text";
const HIDE_QUOTED: &str = "Hide quoted text";

pub enum Choice {
    Web { images: bool },
    Open(usize),
    Save(usize),
    Play(usize),
    Seek(usize, f32),
    ReplyTo,
    Forward,
    Copy,
    Quoted,
    React(&'static str),
    Join(String),
    AddToCalendar,
}

pub const REACTIONS: [&str; 6] = ["👍", "👎", "❤", "😂", "‼", "❓"];
const THUMB: f32 = 240.0;
const PLAYER: f32 = 280.0;

pub enum Thumb {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

pub fn thumb_key(message: &Message, index: usize) -> String {
    format!("{}#{index}", key_of(message))
}

fn is_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp"]
        .iter()
        .any(|extension| lower.ends_with(extension))
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
    pub thumbs: &'a HashMap<String, Thumb>,
    pub wanted: &'a RefCell<Vec<(Message, usize)>>,
    pub pending: Option<(&'a str, &'a str)>,
    pub clock: Clock,
    pub found: &'a [usize],
    pub current: Option<usize>,
    pub scroll: bool,
    pub faces: &'a Faces,
    pub sound: Option<Sound<'a>>,
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
    faces: &'a Faces,
    sound: Option<&'a Sound<'a>>,
}

fn reactions_of(
    conversation: &Conversation,
    pending: Option<(&str, &str)>,
    me: &str,
) -> HashMap<String, Vec<String>> {
    let mut latest: HashMap<(String, String), (chrono::DateTime<chrono::Utc>, String)> =
        HashMap::new();
    for message in &conversation.messages {
        let (Some(emoji), Some(target), Some(from)) =
            (&message.reaction, &message.in_reply_to, &message.from)
        else {
            continue;
        };
        let key = (target.clone(), from.address.to_lowercase());
        if latest
            .get(&key)
            .is_none_or(|(date, _)| *date <= message.date)
        {
            latest.insert(key, (message.date, emoji.clone()));
        }
    }
    if let Some((target, emoji)) = pending {
        latest.insert(
            (target.to_string(), me.to_lowercase()),
            (chrono::Utc::now(), emoji.to_string()),
        );
    }
    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    for ((target, _), (_, emoji)) in latest {
        found.entry(target).or_default().push(emoji);
    }
    found
}

pub struct Action {
    pub message: Message,
    pub choice: Choice,
}

pub fn show(
    ui: &mut Ui,
    conversation: &Conversation,
    me: &Account,
    today: NaiveDate,
    clock: Clock,
    palette: &Palette,
    view: &View<'_>,
) -> Option<Action> {
    let style = Style {
        palette,
        images_by_default: view.images_by_default,
        expanded: view.expanded,
        faces: view.faces,
        sound: view.sound.as_ref(),
    };
    let reactions = reactions_of(conversation, view.pending, &me.address);
    let mut action = None;
    let group = conversation.members.len() > 1;
    let widest = (ui.available_width() * 0.72).min(560.0);
    let mut previous: Option<&Message> = None;
    let mut people_before: Option<BTreeSet<String>> = None;
    for (index, message) in conversation.messages.iter().enumerate() {
        if message.reaction.is_some() {
            continue;
        }
        let mark = if view.current == Some(index) {
            Mark::Current {
                scroll: view.scroll,
            }
        } else if view.found.contains(&index) {
            Mark::Found
        } else {
            Mark::Plain
        };
        let mine = message
            .from
            .as_ref()
            .is_none_or(|from| me.owns(&from.address));
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
        let mut addressed = None;
        if let Some(before) = people_before.as_ref().filter(|before| **before != present) {
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
            let answers_previous = message.in_reply_to.is_none()
                || message.in_reply_to.as_ref()
                    == previous.and_then(|previous| previous.message_id.as_ref());
            let among_several = before.len() > 1 || present.len() > 1;
            if answers_previous && among_several {
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
            } else if mine && !present.is_empty() {
                addressed = Some(format!("to {}", names(present.iter().collect())));
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
        if let Some(addressed) = addressed {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.add_space(EDGE + PADDING.x);
                    ui.label(RichText::new(addressed).size(11.5).color(palette.secondary));
                });
            });
            ui.add_space(2.0);
        } else {
            ui.add_space(if same_sender { 2.0 } else { 6.0 });
        }
        let (choice, shape) = bubble(
            ui,
            message,
            mine,
            widest,
            &style,
            (!mine).then_some(!same_sender || !group),
            mark,
        );
        if let Some(emojis) = message.message_id.as_ref().and_then(|id| reactions.get(id)) {
            badge(ui, shape, mine, emojis, palette);
        }
        let extra = extras(ui, message, mine, shape, view, palette);
        if let Some(choice) = choice.or(extra) {
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

fn people_of(message: &Message, me: &Account) -> BTreeSet<String> {
    message
        .from
        .iter()
        .chain(&message.to)
        .chain(&message.cc)
        .filter(|person| !me.owns(&person.address))
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
    file_menu(&chip, index).or(chip.clicked().then_some(Choice::Open(index)))
}

fn file_menu(chip: &egui::Response, index: usize) -> Option<Choice> {
    let mut chosen = None;
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

fn player(
    ui: &mut Ui,
    spot: Rect,
    name: &str,
    index: usize,
    sound: Option<&Sound<'_>>,
    color: Color32,
) -> Option<Choice> {
    ui.painter()
        .rect_filled(spot, CornerRadius::same(10), color.gamma_multiply(0.12));
    let row = ui
        .interact(
            spot,
            ui.scope_id().with(("player", name, index)),
            Sense::click(),
        )
        .on_hover_text(format!(
            "{name}: click to play, right-click to open or save"
        ));
    let mut chosen = file_menu(&row, index).or(row.clicked().then_some(Choice::Play(index)));
    let symbol = match sound {
        Some(sound) if sound.loading => "…",
        Some(sound) if sound.playing => "⏸",
        _ => "▶",
    };
    let button = Rect::from_min_size(spot.min, vec2(CHIP + 4.0, CHIP));
    let play = egui::Button::new(RichText::new(symbol).size(14.0).color(color)).frame(false);
    if ui.put(button, play).clicked() {
        chosen = Some(Choice::Play(index));
    }
    let position = sound.map_or(Duration::ZERO, |sound| sound.position);
    let duration = sound.and_then(|sound| sound.duration);
    let time = match duration {
        Some(duration) => format!("{} / {}", audio::clock(position), audio::clock(duration)),
        None if position > Duration::ZERO => audio::clock(position),
        None => name.to_string(),
    };
    let time_width = label_width(ui, &time, color).min(spot.width() * 0.5);
    let time_rect = Rect::from_min_max(
        pos2(spot.right() - time_width - PADDING.x, spot.top()),
        pos2(spot.right() - PADDING.x, spot.bottom()),
    );
    ui.put(
        time_rect,
        egui::Label::new(
            RichText::new(time)
                .size(12.0)
                .color(color.gamma_multiply(0.85)),
        )
        .truncate()
        .selectable(false),
    );
    let bar = Rect::from_min_max(
        pos2(button.right() + 2.0, spot.center().y - 2.0),
        pos2(time_rect.left() - PADDING.x, spot.center().y + 2.0),
    );
    if bar.width() > 8.0 {
        let fraction = duration
            .filter(|duration| !duration.is_zero())
            .map_or(0.0, |duration| {
                (position.as_secs_f32() / duration.as_secs_f32()).min(1.0)
            });
        let painter = ui.painter();
        painter.rect_filled(bar, CornerRadius::same(2), color.gamma_multiply(0.3));
        let mut done = bar;
        done.max.x = bar.left() + bar.width() * fraction;
        painter.rect_filled(done, CornerRadius::same(2), color);
        let seek = ui.interact(
            bar.expand2(vec2(0.0, 8.0)),
            ui.scope_id().with(("seek", name, index)),
            Sense::click(),
        );
        if seek.clicked()
            && let Some(pointer) = seek.interact_pointer_pos()
        {
            chosen = Some(Choice::Seek(index, (pointer.x - bar.left()) / bar.width()));
        }
    }
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

fn badge(ui: &Ui, shape: Rect, mine: bool, emojis: &[String], palette: &Palette) {
    let mut counted: Vec<(String, usize)> = Vec::new();
    for emoji in emojis {
        match counted.iter_mut().find(|(known, _)| known == emoji) {
            Some((_, count)) => *count += 1,
            None => counted.push((emoji.clone(), 1)),
        }
    }
    let text: String = counted
        .iter()
        .map(|(emoji, count)| {
            if *count > 1 {
                format!("{emoji}{count}")
            } else {
                emoji.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let galley = ui
        .painter()
        .layout_no_wrap(text, FontId::proportional(18.0), palette.text);
    let size = galley.size() + vec2(14.0, 6.0);
    let corner = if mine {
        pos2(shape.left() - size.x * 0.4, shape.top() - size.y * 0.55)
    } else {
        pos2(shape.right() - size.x * 0.6, shape.top() - size.y * 0.55)
    };
    let rect = Rect::from_min_size(corner, size);
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(10), palette.field);
    painter.rect_stroke(
        rect,
        CornerRadius::same(10),
        egui::Stroke::new(1.0, palette.separator),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.min + vec2(7.0, 3.0), galley, palette.text);
}

fn when(moment: &When, clock: Clock) -> String {
    let value = moment.value.trim();
    if value.len() == 8 {
        return chrono::NaiveDate::parse_from_str(value, "%Y%m%d")
            .map(|date| format!("{} (all day)", date.format("%a %-d %b")))
            .unwrap_or_else(|_| value.to_string());
    }
    let time = |naive: chrono::NaiveDateTime| {
        let hours = match clock {
            Clock::TwelveHour => naive.format("%-I:%M %p").to_string(),
            Clock::TwentyFourHour => naive.format("%H:%M").to_string(),
        };
        format!("{} {hours}", naive.format("%a %-d %b"))
    };
    if let Some(utc) = value.strip_suffix('Z') {
        return chrono::NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S")
            .map(|naive| time(naive.and_utc().with_timezone(&Local).naive_local()))
            .unwrap_or_else(|_| value.to_string());
    }
    let shown = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
        .map(time)
        .unwrap_or_else(|_| value.to_string());
    match &moment.zone {
        Some(zone) => format!("{shown} ({zone})"),
        None => shown,
    }
}

fn extras(
    ui: &mut Ui,
    message: &Message,
    mine: bool,
    shape: Rect,
    view: &View<'_>,
    palette: &Palette,
) -> Option<Choice> {
    let mut chosen = None;
    let place = |ui: &mut Ui, width: f32, add: &mut dyn FnMut(&mut Ui)| {
        ui.horizontal(|ui| {
            if mine {
                ui.add_space((shape.right() - ui.min_rect().left() - width).max(0.0));
            } else {
                ui.add_space((shape.left() - ui.min_rect().left()).max(0.0));
            }
            ui.vertical(|ui| add(ui));
        });
    };
    if let Some(invite) = &message.invite {
        ui.add_space(4.0);
        if invite.method == "REPLY" {
            if let Some((who, answer)) = &invite.answer {
                let said = match answer.as_str() {
                    "ACCEPTED" => "accepted",
                    "DECLINED" => "declined",
                    "TENTATIVE" => "said maybe to",
                    _ => "answered",
                };
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format!("{who} {said} {}", invite.summary))
                            .size(11.5)
                            .italics()
                            .color(palette.secondary),
                    );
                });
            }
        } else {
            place(ui, 320.0, &mut |ui| {
                egui::Frame::new()
                    .fill(palette.field)
                    .stroke(egui::Stroke::new(1.0, palette.separator))
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(300.0);
                        if invite.cancelled {
                            ui.label(
                                RichText::new("Cancelled")
                                    .size(11.5)
                                    .strong()
                                    .color(palette.failure),
                            );
                        } else if invite.sequence > 0 {
                            ui.label(
                                RichText::new("Updated")
                                    .size(11.5)
                                    .strong()
                                    .color(palette.accent),
                            );
                        }
                        ui.label(
                            RichText::new(&invite.summary)
                                .size(14.0)
                                .strong()
                                .color(palette.text),
                        );
                        if let Some(start) = &invite.start {
                            let span = match &invite.end {
                                Some(end) => format!(
                                    "{} – {}",
                                    when(start, view.clock),
                                    when(end, view.clock)
                                ),
                                None => when(start, view.clock),
                            };
                            ui.label(RichText::new(span).size(12.5).color(palette.text));
                        }
                        if !invite.location.is_empty() {
                            ui.label(
                                RichText::new(&invite.location)
                                    .size(12.0)
                                    .color(palette.secondary),
                            );
                        }
                        if !invite.organizer.is_empty() {
                            ui.label(
                                RichText::new(format!("From {}", invite.organizer))
                                    .size(12.0)
                                    .color(palette.secondary),
                            );
                        }
                        ui.horizontal(|ui| {
                            if let Some(link) = &invite.join
                                && !invite.cancelled
                                && ui.button("Join").on_hover_text(link).clicked()
                            {
                                chosen = Some(Choice::Join(link.clone()));
                            }
                            if !invite.cancelled && ui.button("Add to Calendar").clicked() {
                                chosen = Some(Choice::AddToCalendar);
                            }
                        });
                    });
            });
        }
    }
    let images: Vec<(usize, &String)> = message
        .attachments
        .iter()
        .enumerate()
        .filter(|(_, name)| is_image(name))
        .collect();
    if images.is_empty() || message.uid == 0 {
        return chosen;
    }
    ui.add_space(4.0);
    let mut wanted = view.wanted.borrow_mut();
    let width = images.len() as f32 * (THUMB + 6.0);
    place(ui, width.min(THUMB * 2.0 + 6.0), &mut |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(THUMB * 2.0 + 12.0);
            for (index, name) in &images {
                match view.thumbs.get(&thumb_key(message, *index)) {
                    Some(Thumb::Ready(texture)) => {
                        let size = texture.size_vec2();
                        let scale = (THUMB / size.x.max(size.y)).min(1.0);
                        let image = egui::Image::new((texture.id(), size * scale))
                            .corner_radius(10.0)
                            .sense(Sense::click());
                        let response = ui
                            .add(image)
                            .on_hover_text(format!("{name}: click to open, right-click to save"));
                        if let Some(choice) = attachment_choice(&response, *index) {
                            chosen = Some(choice);
                        }
                    }
                    Some(Thumb::Loading) => {
                        ui.add_sized(vec2(THUMB * 0.5, THUMB * 0.35), egui::Spinner::new());
                    }
                    Some(Thumb::Failed) => {}
                    None => {
                        if ui.is_rect_visible(shape.expand(400.0)) {
                            wanted.push((message.clone(), *index));
                        }
                        ui.add_sized(vec2(THUMB * 0.5, THUMB * 0.35), egui::Spinner::new());
                    }
                }
            }
        });
    });
    chosen
}

fn bubble(
    ui: &mut Ui,
    message: &Message,
    mine: bool,
    widest: f32,
    style: &Style<'_>,
    avatar: Option<bool>,
    mark: Mark,
) -> (Option<Choice>, Rect) {
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
        label_width(ui, WEB_VIEW, tone) + label_width(ui, SHOW_IMAGES, tone) + 3.0 * CONTROL_GAP
    } else {
        label_width(ui, WEB_VIEW, tone)
    };
    let controls_height = if message.html {
        CONTROLS + CHIP_GAP
    } else {
        0.0
    };
    let fold = (message.quoted && has_text).then_some(if full.is_some() {
        HIDE_QUOTED
    } else {
        SHOW_QUOTED
    });
    let fold_height = if fold.is_some() {
        CONTROLS + CHIP_GAP
    } else {
        0.0
    };
    let fold_width = fold.map_or(0.0, |label| label_width(ui, label, tone));
    let chips_width = message
        .attachments
        .iter()
        .map(|name| {
            if is_audio(name) {
                PLAYER
            } else {
                label_width(ui, &format!("📎 {name}"), color) + 2.0 * PADDING.x
            }
        })
        .fold(0.0, f32::max);
    let chips_height = message.attachments.len() as f32 * (CHIP + CHIP_GAP)
        - if has_text || message.attachments.is_empty() {
            0.0
        } else {
            CHIP_GAP
        };
    let size = vec2(
        text.x
            .max(chips_width)
            .max(controls_width)
            .max(fold_width)
            .min(inner),
        controls_height + text.y + fold_height + chips_height,
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
        list::avatar(ui, spot, message.from.as_ref(), style.faces);
    }
    let mut area = ui.interact(
        shape,
        ui.scope_id()
            .with(("bubble", message.uid, &message.message_id)),
        Sense::click(),
    );
    let mut chosen = None;
    let inside = shape.shrink2(PADDING);
    let mut top = inside.top();
    let control =
        |text: &str| egui::Button::new(RichText::new(text).size(13.0).color(tone)).frame(false);
    if message.html {
        let web_width = label_width(ui, WEB_VIEW, tone);
        let images_width = label_width(ui, SHOW_IMAGES, tone);
        let web = Rect::from_min_size(pos2(inside.left(), top), vec2(web_width, CONTROLS));
        let images = Rect::from_min_size(
            pos2(inside.right() - images_width, top),
            vec2(images_width, CONTROLS),
        );
        let hint = if separate_images {
            "Show this message as the sender laid it out, without loading pictures from the sender's servers"
        } else {
            "Show this message as the sender laid it out, with its pictures"
        };
        if ui.put(web, control(WEB_VIEW)).on_hover_text(hint).clicked() {
            chosen = Some(Choice::Web {
                images: images_by_default,
            });
        }
        if separate_images {
            let middle = (web.right() + images.left()) / 2.0;
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
                chosen = Some(Choice::Web { images: true });
            }
        }
        top += CONTROLS + CHIP_GAP;
    }
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
    if let Some(label) = fold {
        let spot = Rect::from_min_size(pos2(inside.left(), top), vec2(fold_width, CONTROLS));
        if ui.put(spot, control(label)).clicked() {
            chosen = Some(Choice::Quoted);
        }
        top += CONTROLS + CHIP_GAP;
    }
    area.context_menu(|ui| {
        if message.message_id.is_some() && message.uid != 0 {
            ui.horizontal(|ui| {
                for emoji in REACTIONS {
                    if ui.button(RichText::new(emoji).size(16.0)).clicked() {
                        chosen = Some(Choice::React(emoji));
                        ui.close();
                    }
                }
            });
            ui.separator();
        }
        let mut item = |ui: &mut Ui, label: &str, choice: Choice| {
            if ui.button(label).clicked() {
                chosen = Some(choice);
                ui.close();
            }
        };
        item(ui, "Reply to This Message", Choice::ReplyTo);
        item(ui, "Forward…", Choice::Forward);
        item(ui, "Copy Text", Choice::Copy);
        if message.quoted {
            let quoted = if full.is_some() {
                "Hide Quoted Text"
            } else {
                "Show Quoted Text"
            };
            item(ui, quoted, Choice::Quoted);
        }
    });
    for (index, name) in message.attachments.iter().enumerate() {
        let spot = Rect::from_min_size(pos2(inside.left(), top), vec2(inside.width(), CHIP));
        let choice = if is_audio(name) {
            let key = thumb_key(message, index);
            let sound = style.sound.filter(|sound| sound.key == key);
            player(ui, spot, name, index, sound, color)
        } else {
            let chip = ui.put(spot, attachment_button(name, color));
            attachment_choice(&chip, index)
        };
        if choice.is_some() {
            chosen = choice;
        }
        top += CHIP + CHIP_GAP;
    }
    (chosen, shape)
}
