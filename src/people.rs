// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::HashMap;

use eframe::egui::{self, Response, TextEdit, Ui};
use zirbe_store::{Account, Conversation, Person, Query};

const SHOWN: usize = 6;

pub fn known(
    conversations: &[Conversation],
    contacts: &[Person],
    accounts: &[Account],
) -> Vec<Person> {
    let mut seen: HashMap<String, (Person, usize, bool)> = HashMap::new();
    for message in conversations.iter().flat_map(|c| &c.messages) {
        let mine = message
            .from
            .as_ref()
            .is_some_and(|from| accounts.iter().any(|a| from.is(&a.address)));
        let people = message.from.iter().chain(&message.to).chain(&message.cc);
        for person in people {
            if accounts.iter().any(|account| person.is(&account.address)) {
                continue;
            }
            let entry = seen
                .entry(person.address.to_lowercase())
                .or_insert_with(|| (person.clone(), 0, false));
            entry.1 += 1;
            entry.2 |= mine;
            if entry.0.name.is_none() {
                entry.0.name = person.name.clone();
            }
        }
    }
    for contact in contacts {
        if accounts.iter().any(|account| contact.is(&account.address)) {
            continue;
        }
        let entry = seen
            .entry(contact.address.to_lowercase())
            .or_insert_with(|| (contact.clone(), 0, false));
        if contact.name.is_some() {
            entry.0.name = contact.name.clone();
        }
    }
    let mut people: Vec<(Person, usize, bool)> = seen.into_values().collect();
    people.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then(b.1.cmp(&a.1))
            .then_with(|| a.0.display().cmp(b.0.display()))
    });
    people.into_iter().map(|(person, _, _)| person).collect()
}

fn written(person: &Person) -> String {
    match &person.name {
        Some(name) if name.contains([',', '"', '<']) => {
            format!("\"{}\" <{}>", name.replace('"', ""), person.address)
        }
        Some(name) => format!("{name} <{}>", person.address),
        None => person.address.clone(),
    }
}

pub fn field(
    ui: &mut Ui,
    text: &mut String,
    hint: &str,
    width: f32,
    salt: &str,
    known: &[Person],
) -> Response {
    let id = ui.make_persistent_id(("people", salt));
    let response = ui.add(
        TextEdit::singleline(text)
            .id(id)
            .hint_text(hint)
            .desired_width(width),
    );
    let hovered_key = id.with("hovered");
    let list_hovered = ui
        .ctx()
        .data(|data| data.get_temp::<bool>(hovered_key))
        .unwrap_or(false);
    let (head, typed) = match text.rfind([',', ';']) {
        Some(at) => (text[..=at].to_string(), text[at + 1..].trim().to_string()),
        None => (String::new(), text.trim().to_string()),
    };
    let query = Query::new(&typed);
    let open = response.has_focus() || response.lost_focus() || list_hovered;
    let chosen_already: Vec<String> = text
        .to_lowercase()
        .split([',', ';'])
        .map(|part| part.trim().to_string())
        .collect();
    let matches: Vec<&Person> = if query.is_empty() || !open {
        Vec::new()
    } else {
        known
            .iter()
            .filter(|person| {
                query.matches_text(&format!(
                    "{} {}",
                    person.name.as_deref().unwrap_or_default(),
                    person.address
                ))
            })
            .filter(|person| {
                !chosen_already
                    .iter()
                    .any(|chosen| chosen.contains(&person.address.to_lowercase()))
            })
            .take(SHOWN)
            .collect()
    };
    if matches.is_empty() {
        ui.ctx()
            .data_mut(|data| data.insert_temp(hovered_key, false));
        return response;
    }
    let mut picked = None;
    let list = egui::Popup::from_response(&response)
        .open(true)
        .width(width)
        .show(|ui| {
            for person in &matches {
                let label = match &person.name {
                    Some(name) => format!("{name}  <{}>", person.address),
                    None => person.address.clone(),
                };
                if ui.selectable_label(false, label).clicked() {
                    picked = Some((*person).clone());
                }
            }
        });
    let hovered = list.is_some_and(|list| list.response.contains_pointer());
    ui.ctx()
        .data_mut(|data| data.insert_temp(hovered_key, hovered && picked.is_none()));
    if let Some(person) = picked {
        let separator = if head.is_empty() { "" } else { " " };
        *text = format!("{head}{separator}{}, ", written(&person));
        ui.memory_mut(|memory| memory.request_focus(id));
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let end = egui::text::CCursor::new(text.chars().count());
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(end)));
            state.store(ui.ctx(), id);
        }
    }
    response
}
