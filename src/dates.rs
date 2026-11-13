// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};

use crate::system::Clock;

pub fn time(at: DateTime<Utc>, clock: Clock) -> String {
    let local = at.with_timezone(&Local);
    match clock {
        Clock::TwelveHour => local.format("%-I:%M %p").to_string(),
        Clock::TwentyFourHour => local.format("%H:%M").to_string(),
    }
}

pub fn stamp(at: DateTime<Utc>, clock: Clock) -> String {
    let date = at.with_timezone(&Local).format("%A %-d %B %Y");
    format!("{date}, {}", time(at, clock))
}

pub fn day(at: DateTime<Utc>, today: NaiveDate) -> String {
    let date = at.with_timezone(&Local).date_naive();
    let days = (today - date).num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => date.format("%A").to_string(),
        _ if date.year() == today.year() => date.format("%A %-d %B").to_string(),
        _ => date.format("%-d %B %Y").to_string(),
    }
}

pub fn list_label(at: DateTime<Utc>, today: NaiveDate, clock: Clock) -> String {
    let date = at.with_timezone(&Local).date_naive();
    match (today - date).num_days() {
        0 => time(at, clock),
        1 => "Yesterday".into(),
        2..=6 => date.format("%A").to_string(),
        _ if date.year() == today.year() => date.format("%-d %b").to_string(),
        _ => date.format("%-d %b %Y").to_string(),
    }
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}
