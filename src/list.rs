// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use chrono::NaiveDate;
use eframe::egui::{self, Align2, FontId, Rect, Response, Sense, Ui, pos2, vec2};
use zirbe_store::{Conversation, Person};

use crate::contacts::Faces;
use crate::dates;
use crate::palette::{self, Palette};
use crate::system::Clock;

pub fn height(lines: u8) -> f32 {
    54.0 + 18.0 * f32::from(lines)
}

pub struct Look<'a> {
    pub lines: u8,
    pub dot: Option<egui::Color32>,
    pub faces: &'a Faces,
}
const AVATAR: f32 = 40.0;

pub fn title(conversation: &Conversation) -> String {
    if !conversation.title.is_empty() {
        return conversation.title.clone();
    }
    match conversation.members.as_slice() {
        [] => "Chat".into(),
        members => format!(
            "Chat with {}",
            members
                .iter()
                .map(Person::short)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub fn members(conversation: &Conversation) -> String {
    match conversation.members.as_slice() {
        [] => "Only you".into(),
        [one] => one.display().to_string(),
        many => many
            .iter()
            .map(Person::short)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

pub fn initials(person: Option<&Person>) -> String {
    let Some(person) = person else {
        return "?".into();
    };
    let source = person.name.as_deref().unwrap_or(&person.address);
    let mut letters = source
        .split(|c: char| c.is_whitespace() || c == '.' || c == '@')
        .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase);
    let first = letters.next().unwrap_or('?');
    match (person.name.is_some(), letters.next()) {
        (true, Some(second)) => format!("{first}{second}"),
        _ => first.to_string(),
    }
}

pub fn avatar(ui: &Ui, rect: Rect, person: Option<&Person>, faces: &Faces) {
    if let Some(face) = faces.of(person) {
        egui::Image::new((face.id(), rect.size()))
            .corner_radius(rect.width() / 2.0)
            .paint_at(ui, rect);
        return;
    }
    let painter = ui.painter();
    let center = rect.center();
    let seed = person.map_or("", |person| person.address.as_str());
    painter.circle_filled(center, rect.width() / 2.0, palette::monogram(seed));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        initials(person),
        FontId::proportional(rect.width() * 0.38),
        egui::Color32::WHITE,
    );
}

fn snippet(conversation: &Conversation, account: &str) -> String {
    let Some(last) = conversation.last() else {
        return String::new();
    };
    let text: String = last.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let text = if text.is_empty() && !last.attachments.is_empty() {
        format!("Attachment: {}", last.attachments.join(", "))
    } else {
        text
    };
    let mine = last.from.as_ref().is_some_and(|from| from.is(account));
    if mine { format!("You: {text}") } else { text }
}

pub fn row(
    ui: &mut Ui,
    conversation: &Conversation,
    selected: bool,
    today: NaiveDate,
    clock: Clock,
    palette: &Palette,
    look: &Look<'_>,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(
        vec2(ui.available_width(), height(look.lines)),
        Sense::click(),
    );
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect.shrink2(vec2(6.0, 2.0)), 10.0, palette.selected);
    } else if response.hovered() {
        painter.rect_filled(
            rect.shrink2(vec2(6.0, 2.0)),
            10.0,
            palette.selected.gamma_multiply(0.45),
        );
    }
    let unread = conversation.unread > 0;
    if unread {
        painter.circle_filled(
            pos2(rect.left() + 13.0, rect.center().y),
            4.5,
            palette.accent,
        );
    }
    let avatar_rect = Rect::from_center_size(
        pos2(rect.left() + 44.0, rect.center().y),
        vec2(AVATAR, AVATAR),
    );
    let lead = conversation
        .last()
        .and_then(|last| last.from.as_ref())
        .filter(|from| {
            conversation
                .members
                .iter()
                .any(|member| member.is(&from.address))
        })
        .or(conversation.members.first());
    avatar(ui, avatar_rect, lead, look.faces);
    if let Some(colour) = look.dot {
        let spot = avatar_rect.right_bottom() - vec2(5.0, 5.0);
        ui.painter().circle_filled(spot, 6.0, palette.sidebar);
        ui.painter().circle_filled(spot, 4.5, colour);
    }
    let left = avatar_rect.right() + 12.0;
    let right = rect.right() - 14.0;
    let date = conversation
        .last()
        .map(|last| dates::list_label(last.date, today, clock))
        .unwrap_or_default();
    let date_galley = painter.layout_no_wrap(date, FontId::proportional(12.0), palette.secondary);
    let date_width = date_galley.size().x;
    painter.galley(
        pos2(right - date_width, rect.top() + 12.0),
        date_galley,
        palette.secondary,
    );
    let line = |text: String, size: f32, color, top: f32, width: f32| {
        let mut job = egui::text::LayoutJob::single_section(
            text,
            egui::TextFormat::simple(FontId::proportional(size), color),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(width);
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        ui.painter()
            .galley(pos2(left, rect.top() + top), galley, color);
    };
    line(
        title(conversation),
        14.5,
        palette.text,
        10.0,
        right - left - date_width - 8.0,
    );
    let mut marks_width = 0.0;
    for (mark, color) in [
        (conversation.flagged, "⚑", palette.flag),
        (conversation.pinned, "📌", palette.secondary),
    ]
    .into_iter()
    .filter(|(shown, _, _)| *shown)
    .map(|(_, mark, color)| (mark, color))
    {
        let galley = painter.layout_no_wrap(mark.into(), FontId::proportional(12.0), color);
        marks_width += galley.size().x + 4.0;
        painter.galley(
            pos2(right - marks_width + 4.0, rect.top() + 30.0),
            galley,
            color,
        );
    }
    line(
        members(conversation),
        12.5,
        palette.secondary,
        30.0,
        right - left - marks_width,
    );
    if look.lines > 0 {
        let color = if unread {
            palette.text
        } else {
            palette.secondary
        };
        let mut job = egui::text::LayoutJob::single_section(
            snippet(conversation, conversation.account.as_str()),
            egui::TextFormat::simple(FontId::proportional(12.5), color),
        );
        job.wrap = egui::text::TextWrapping {
            max_width: right - left,
            max_rows: usize::from(look.lines),
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        ui.painter()
            .galley(pos2(left, rect.top() + 48.0), galley, color);
    }
    response
}
