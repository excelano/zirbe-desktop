// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use chrono::NaiveDate;
use eframe::egui::{self, Align2, FontId, Key, Rect, Response, Sense, Ui, pos2, vec2};
use zirbe_store::{Conversation, Person};

use crate::contacts::Faces;
use crate::dates;
use crate::media;
use crate::palette::{self, Palette};
use crate::system::Clock;

pub fn height(lines: u8) -> f32 {
    54.0 + 18.0 * f32::from(lines)
}

pub fn page(visible: f32, row: f32) -> usize {
    ((visible / row) as usize).max(1)
}

pub fn moves(page: usize) -> [(Key, isize); 6] {
    let page = page as isize;
    [
        (Key::ArrowDown, 1),
        (Key::ArrowUp, -1),
        (Key::PageDown, page),
        (Key::PageUp, -page),
        (Key::End, isize::MAX),
        (Key::Home, isize::MIN),
    ]
}

pub fn step(at: Option<usize>, by: isize, rows: usize) -> Option<usize> {
    let last = rows.checked_sub(1)?;
    let next = match at {
        Some(at) => at.saturating_add_signed(by).min(last),
        None if by > 0 => (by.unsigned_abs() - 1).min(last),
        None => 0,
    };
    (at != Some(next)).then_some(next)
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
    let transcript = last.transcript.as_deref().unwrap_or_default().trim();
    let text = if text.is_empty() && !transcript.is_empty() {
        transcript.to_string()
    } else if text.is_empty() && !last.attachments.is_empty() {
        format!("Attachment: {}", last.attachments.join(", "))
    } else {
        text
    };
    let mine = last.from.as_ref().is_some_and(|from| from.is(account));
    let glyph = media::glyph(conversation);
    if mine {
        format!("{glyph}You: {text}")
    } else {
        format!("{glyph}{text}")
    }
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

#[cfg(test)]
mod tests {
    use super::{Key, moves, page, step};

    fn by(key: Key, rows_on_screen: usize) -> isize {
        moves(rows_on_screen)
            .into_iter()
            .find(|(candidate, _)| *candidate == key)
            .map(|(_, by)| by)
            .expect("a move for the key")
    }

    #[test]
    fn a_page_is_the_rows_that_fit_whole_and_never_less_than_one() {
        assert_eq!(page(380.0, 93.0), 4);
        assert_eq!(page(372.0, 93.0), 4);
        assert_eq!(page(371.9, 93.0), 3);
        assert_eq!(page(40.0, 93.0), 1);
    }

    #[test]
    fn arrows_move_one_row_and_stop_at_either_end() {
        assert_eq!(step(Some(3), by(Key::ArrowDown, 4), 10), Some(4));
        assert_eq!(step(Some(3), by(Key::ArrowUp, 4), 10), Some(2));
        assert_eq!(step(Some(9), by(Key::ArrowDown, 4), 10), None);
        assert_eq!(step(Some(0), by(Key::ArrowUp, 4), 10), None);
    }

    #[test]
    fn page_keys_move_a_screenful_and_stop_at_the_ends() {
        assert_eq!(step(Some(0), by(Key::PageDown, 4), 30), Some(4));
        assert_eq!(step(Some(8), by(Key::PageUp, 4), 30), Some(4));
        assert_eq!(step(Some(27), by(Key::PageDown, 4), 30), Some(29));
        assert_eq!(step(Some(2), by(Key::PageUp, 4), 30), Some(0));
        assert_eq!(step(Some(29), by(Key::PageDown, 4), 30), None);
        assert_eq!(step(Some(0), by(Key::PageUp, 4), 30), None);
    }

    #[test]
    fn home_and_end_go_to_the_first_and_last_row() {
        assert_eq!(step(Some(12), by(Key::End, 4), 30), Some(29));
        assert_eq!(step(Some(12), by(Key::Home, 4), 30), Some(0));
        assert_eq!(step(Some(29), by(Key::End, 4), 30), None);
        assert_eq!(step(Some(0), by(Key::Home, 4), 30), None);
    }

    #[test]
    fn with_nothing_selected_a_move_starts_from_the_top() {
        assert_eq!(step(None, by(Key::ArrowDown, 4), 30), Some(0));
        assert_eq!(step(None, by(Key::ArrowUp, 4), 30), Some(0));
        assert_eq!(step(None, by(Key::PageDown, 4), 30), Some(3));
        assert_eq!(step(None, by(Key::PageUp, 4), 30), Some(0));
        assert_eq!(step(None, by(Key::Home, 4), 30), Some(0));
        assert_eq!(step(None, by(Key::End, 4), 30), Some(29));
        assert_eq!(step(None, by(Key::PageDown, 4), 2), Some(1));
    }

    #[test]
    fn an_empty_list_or_a_lone_selected_row_goes_nowhere() {
        for (_, by) in moves(4) {
            assert_eq!(step(None, by, 0), None);
            assert_eq!(step(Some(0), by, 1), None);
        }
    }
}
