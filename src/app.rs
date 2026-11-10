// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use eframe::egui::{
    self, Align, Frame, Key, Layout, Margin, Modifiers, Rect, RichText, ScrollArea, TextEdit, Ui,
    UiBuilder, pos2,
};
use zirbe_store::{
    Account, Change, Conversation, Draft, FolderInfo, INBOX, MailSource, Message, Query, Reply,
    Role, Scope, Status, Store,
};

use crate::bubbles::{self, Action, Choice};
use crate::dates;
use crate::demo::Demo;
use crate::files;
use crate::list;
use crate::notify::Notifier;
use crate::page::Page;
use crate::palette::Palette;
use crate::settings::Settings;
use crate::signin::{Outcome, SignIn};
use crate::system::Clock;

const SEARCH: &str = "conversation search";

pub enum Source {
    Live(Store),
    Demo(Demo),
}

impl Source {
    fn mail(&self) -> &dyn MailSource {
        match self {
            Source::Live(store) => store,
            Source::Demo(demo) => demo,
        }
    }
}

enum Pane {
    Conversation,
    New(NewConversation),
}

#[derive(Default)]
struct NewConversation {
    from: String,
    to: String,
    name: String,
    body: String,
    pending: Option<Reply>,
    failure: Option<String>,
}

struct Sending {
    conversation: String,
    reply: Reply,
}

struct Fetching {
    conversation: String,
    work: Work,
}

struct ServerSearch {
    query: String,
    reply: Option<Reply<usize>>,
    result: Option<Result<usize, String>>,
}

struct Find {
    conversation: String,
    query: String,
    current: usize,
    scroll: bool,
    focus: bool,
}

struct Reading {
    conversation: String,
    account: String,
    message: Message,
    images: bool,
    shown: Shown,
}

enum Shown {
    Loading(Reply<String>),
    Ready { html: String, page: Page },
    Failed(String),
}

enum Work {
    Open { name: String, reply: Reply<Vec<u8>> },
    Save { name: String, reply: Reply<Vec<u8>> },
    Saving { name: String, reply: Reply },
}

impl Work {
    fn doing(&self) -> String {
        match self {
            Work::Open { name, .. } | Work::Save { name, .. } => format!("Downloading {name}…"),
            Work::Saving { name, .. } => format!("Saving {name}…"),
        }
    }

    fn failed(&self, error: &str) -> String {
        match self {
            Work::Open { name, .. } => format!("{name} did not open: {error}"),
            Work::Save { name, .. } | Work::Saving { name, .. } => {
                format!("{name} was not saved: {error}")
            }
        }
    }
}

fn arrived<T>(reply: &Reply<T>, ctx: &egui::Context) -> Option<Result<T, String>> {
    match reply.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => {
            ctx.request_repaint_after(Duration::from_millis(150));
            None
        }
        Err(TryRecvError::Disconnected) => Some(Err("the mail engine stopped".into())),
    }
}

pub struct App {
    source: Source,
    palette: Palette,
    clock: Clock,
    selected: Option<String>,
    drafts: HashMap<String, String>,
    sending: Option<Sending>,
    fetching: Option<Fetching>,
    reading: Option<Reading>,
    failure: Option<(String, String)>,
    pane: Pane,
    scope: Scope,
    settings: Settings,
    settings_open: bool,
    picked: BTreeSet<String>,
    anchor: Option<String>,
    shown: Vec<String>,
    unread_only: bool,
    confirm_delete: Option<Vec<Conversation>>,
    notifier: Option<Notifier>,
    query: String,
    matches: Option<(String, usize, BTreeSet<String>)>,
    server_search: Option<ServerSearch>,
    find: Option<Find>,
    auto_opened: Option<String>,
    adding_account: bool,
    sign_in: SignIn,
    marked: Option<(String, usize)>,
    #[cfg(feature = "screenshot")]
    screenshot: Option<crate::screenshot::Screenshot>,
}

impl App {
    pub fn new(source: Source, palette: Palette, clock: Clock) -> App {
        App {
            source,
            palette,
            clock,
            selected: None,
            drafts: HashMap::new(),
            sending: None,
            fetching: None,
            reading: None,
            failure: None,
            pane: Pane::Conversation,
            scope: Scope::AllInboxes,
            settings: Settings::load(),
            settings_open: false,
            picked: BTreeSet::new(),
            anchor: None,
            shown: Vec::new(),
            unread_only: false,
            confirm_delete: None,
            notifier: None,
            query: String::new(),
            matches: None,
            server_search: None,
            find: None,
            auto_opened: None,
            adding_account: false,
            sign_in: SignIn::default(),
            marked: None,
            #[cfg(feature = "screenshot")]
            screenshot: crate::screenshot::Screenshot::from_environment(),
        }
    }

    #[cfg(feature = "screenshot")]
    pub fn open_for_screenshot(&mut self, opening: &str) {
        match opening {
            "new" => self.start_new(),
            "settings" => self.settings_open = true,
            title => {
                self.selected = self
                    .source
                    .mail()
                    .conversations()
                    .iter()
                    .find(|conversation| conversation.title == title)
                    .map(|conversation| conversation.key.clone());
            }
        }
    }

    fn start_new(&mut self) {
        let from = self
            .source
            .mail()
            .accounts()
            .first()
            .map(Account::key)
            .unwrap_or_default();
        self.pane = Pane::New(NewConversation {
            from,
            ..NewConversation::default()
        });
    }

    fn account(&self, key: &str) -> Option<Account> {
        self.source
            .mail()
            .accounts()
            .into_iter()
            .find(|account| account.key() == key)
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::N)) {
            self.start_new();
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::R)) {
            self.source.mail().refresh();
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F)) {
            self.open_find();
        }
        if ctx.input_mut(|input| input.consume_key(Modifiers::COMMAND, Key::F)) {
            ctx.memory_mut(|memory| memory.request_focus(egui::Id::unique(SEARCH)));
        }
        if self.find.is_some()
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.find = None;
        }
        if matches!(self.pane, Pane::New(_))
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.pane = Pane::Conversation;
        }
        if self.reading.is_some()
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.reading = None;
        }
    }

    fn poll_sending(&mut self, ctx: &egui::Context) {
        let Some(sending) = &self.sending else { return };
        match sending.reply.try_recv() {
            Ok(Ok(())) => {
                self.drafts.remove(&sending.conversation);
                self.sending = None;
            }
            Ok(Err(error)) => {
                self.failure = Some((sending.conversation.clone(), format!("Not sent. {error}")));
                self.sending = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(150)),
            Err(TryRecvError::Disconnected) => self.sending = None,
        }
    }

    fn poll_fetching(&mut self, ctx: &egui::Context) {
        let Some(Fetching { conversation, work }) = self.fetching.take() else {
            return;
        };
        let failed = |error: String| work.failed(&error);
        let next = match &work {
            Work::Open { name, reply } => arrived(reply, ctx).map(|result| {
                result
                    .and_then(|bytes| files::open(name, &bytes))
                    .map(|()| None)
            }),
            Work::Save { name, reply } => arrived(reply, ctx).map(|result| {
                result.map(|bytes| {
                    Some(Work::Saving {
                        name: name.clone(),
                        reply: files::save(name.clone(), bytes),
                    })
                })
            }),
            Work::Saving { reply, .. } => arrived(reply, ctx).map(|result| result.map(|()| None)),
        };
        match next {
            None => self.fetching = Some(Fetching { conversation, work }),
            Some(Ok(Some(work))) => self.fetching = Some(Fetching { conversation, work }),
            Some(Ok(None)) => {}
            Some(Err(error)) => self.failure = Some((conversation, failed(error))),
        }
    }

    fn sidebar(&mut self, ui: &mut Ui, conversations: &[Conversation]) {
        let palette = self.palette;
        egui::Panel::left("conversations")
            .resizable(true)
            .default_size(340.0)
            .size_range(260.0..=520.0)
            .frame(
                Frame::new()
                    .fill(palette.sidebar)
                    .inner_margin(Margin::symmetric(0, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    if let Some(scope) = self.switcher(ui)
                        && scope != self.scope
                    {
                        self.source.mail().open(&scope);
                        if self.selected.as_ref().is_some_and(|key| {
                            !conversations.iter().any(|conversation| {
                                &conversation.key == key && scope.holds(conversation)
                            })
                        }) {
                            self.selected = None;
                        }
                        self.scope = scope;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(10.0);
                        if ui.button("⚙").on_hover_text("Settings").clicked() {
                            self.settings_open = true;
                        }
                        if ui
                            .button("✏")
                            .on_hover_text("New conversation (Ctrl+N)")
                            .clicked()
                        {
                            self.start_new();
                        }
                    });
                });
                ui.add_space(6.0);
                self.search_field(ui);
                ui.add_space(4.0);
                egui::Panel::bottom("status")
                    .frame(
                        Frame::new()
                            .fill(palette.sidebar)
                            .inner_margin(Margin::symmetric(14, 6)),
                    )
                    .show(ui, |ui| self.status(ui));
                let shown: Vec<&Conversation> = conversations
                    .iter()
                    .filter(|conversation| self.scope.holds(conversation))
                    .filter(|conversation| {
                        self.matches
                            .as_ref()
                            .is_none_or(|(_, _, keys)| keys.contains(&conversation.key))
                    })
                    .filter(|conversation| {
                        !self.unread_only
                            || conversation.unread > 0
                            || self.selected.as_ref() == Some(&conversation.key)
                            || self.picked.contains(&conversation.key)
                    })
                    .collect();
                self.shown = shown.iter().map(|c| c.key.clone()).collect();
                if shown.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        let text = if self.matches.is_some() {
                            "No matches here"
                        } else if self.unread_only {
                            "You're all caught up"
                        } else {
                            "No conversations here"
                        };
                        ui.label(RichText::new(text).size(13.0).color(palette.secondary));
                    });
                }
                let mut pending = None;
                ScrollArea::vertical().auto_shrink(false).show_rows(
                    ui,
                    list::ROW_HEIGHT,
                    shown.len(),
                    |ui, range| {
                        let today = dates::today();
                        for conversation in &shown[range] {
                            let key = &conversation.key;
                            let selected = matches!(self.pane, Pane::Conversation)
                                && (self.selected.as_ref() == Some(key)
                                    || self.picked.contains(key));
                            let response =
                                list::row(ui, conversation, selected, today, self.clock, &palette);
                            if response.clicked() {
                                let modifiers = ui.input(|input| input.modifiers);
                                self.pick(key, modifiers);
                            }
                            response.context_menu(|ui| {
                                let targets: Vec<Conversation> =
                                    if self.picked.len() > 1 && self.picked.contains(key) {
                                        shown
                                            .iter()
                                            .filter(|c| self.picked.contains(&c.key))
                                            .map(|c| (*c).clone())
                                            .collect()
                                    } else {
                                        vec![(*conversation).clone()]
                                    };
                                if let Some(change) = self.actions_menu(ui, &targets) {
                                    pending = Some((targets, change));
                                }
                            });
                        }
                    },
                );
                if let Some((targets, change)) = pending {
                    self.apply(targets, change);
                }
            });
    }

    fn news(&mut self, ctx: &egui::Context, conversations: &[Conversation]) {
        let notifier = self.notifier.get_or_insert_with(|| Notifier::new(ctx));
        let arrivals = self.source.mail().arrivals();
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(false));
        if self.settings.notify && !focused && !arrivals.is_empty() {
            notifier.post(&arrivals);
        }
        let unread: u32 = self
            .source
            .mail()
            .accounts()
            .iter()
            .flat_map(|account| self.source.mail().folders(&account.key()))
            .filter(|folder| folder.role == Some(Role::Inbox))
            .map(|folder| folder.unread)
            .sum();
        notifier.badge(unread);
        let Some(target) = notifier.clicked() else {
            return;
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        let found = target.and_then(|id| {
            conversations.iter().find(|conversation| {
                conversation
                    .copies
                    .iter()
                    .any(|copy| copy.message_id.as_ref() == Some(&id))
            })
        });
        if let Some(conversation) = found {
            if !self.scope.holds(conversation) {
                self.scope = Scope::AllInboxes;
                self.source.mail().open(&self.scope);
            }
            self.selected = Some(conversation.key.clone());
            self.picked.clear();
            self.pane = Pane::Conversation;
        }
    }

    fn search_field(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            let field = ui.add(
                TextEdit::singleline(&mut self.query)
                    .id(egui::Id::unique(SEARCH))
                    .hint_text("Search (Ctrl+F)")
                    .desired_width(ui.available_width() - 40.0),
            );
            if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Escape)) {
                self.query.clear();
            }
            if !self.query.is_empty() && ui.small_button("✕").on_hover_text("Clear").clicked() {
                self.query.clear();
            }
        });
        if Query::new(&self.query).is_empty() {
            return;
        }
        let place = match &self.scope {
            Scope::AllInboxes => "Inboxes".to_string(),
            Scope::Mailbox { .. } => self.scope_label(),
        };
        let query = self.query.trim().to_string();
        let state = match self
            .server_search
            .as_ref()
            .filter(|search| search.query == query)
        {
            Some(ServerSearch { reply: Some(_), .. }) => Some(None),
            Some(ServerSearch {
                result: Some(result),
                ..
            }) => Some(Some(match result {
                Ok(0) => "Nothing more on the server".to_string(),
                Ok(count) => format!("Found {count} more on the server"),
                Err(error) => format!("Server search failed: {error}"),
            })),
            _ => None,
        };
        let mut start = false;
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            match &state {
                Some(None) => {
                    ui.spinner();
                    ui.label(
                        RichText::new("Searching the server…")
                            .size(12.0)
                            .color(palette.secondary),
                    );
                }
                Some(Some(text)) => {
                    ui.label(RichText::new(text).size(12.0).color(palette.secondary));
                }
                None => {
                    let link = egui::Button::new(
                        RichText::new(format!("Search {place} on Server"))
                            .size(12.0)
                            .color(palette.accent),
                    )
                    .frame(false);
                    start = ui
                        .add(link)
                        .on_hover_text("Ask the mail server for older messages that match")
                        .clicked();
                }
            }
        });
        if start {
            self.server_search = Some(ServerSearch {
                reply: Some(self.source.mail().search_server(&self.scope, &query)),
                query,
                result: None,
            });
        }
        if let Some(search) = &mut self.server_search
            && let Some(reply) = &search.reply
            && let Some(result) = arrived(reply, ui.ctx())
        {
            search.result = Some(result);
            search.reply = None;
        }
    }

    fn update_matches(&mut self, conversations: &Arc<Vec<Conversation>>) {
        let query = Query::new(&self.query);
        if query.is_empty() {
            self.matches = None;
            return;
        }
        let source = Arc::as_ptr(conversations) as usize;
        if self
            .matches
            .as_ref()
            .is_some_and(|(text, from, _)| *text == self.query && *from == source)
        {
            return;
        }
        let keys = conversations
            .iter()
            .filter(|conversation| query.matches(conversation))
            .map(|conversation| conversation.key.clone())
            .collect();
        self.matches = Some((self.query.clone(), source, keys));
    }

    fn open_find(&mut self) {
        if let (Pane::Conversation, Some(key)) = (&self.pane, &self.selected)
            && self.picked.len() < 2
        {
            self.reading = None;
            self.find = Some(Find {
                conversation: key.clone(),
                query: String::new(),
                current: 0,
                scroll: false,
                focus: true,
            });
        }
    }

    fn find_bar(&mut self, ui: &mut Ui, conversation: &Conversation) -> Vec<usize> {
        let palette = self.palette;
        let Some(find) = &mut self.find else {
            return Vec::new();
        };
        let query = Query::new(&find.query);
        let found: Vec<usize> = if query.is_empty() {
            Vec::new()
        } else {
            conversation
                .messages
                .iter()
                .enumerate()
                .filter(|(_, message)| query.matches_message(message))
                .map(|(index, _)| index)
                .collect()
        };
        find.scroll = false;
        let mut close = false;
        egui::Panel::top("find")
            .frame(
                Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(16, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let field = ui.add(
                        TextEdit::singleline(&mut find.query)
                            .hint_text("Find in conversation")
                            .desired_width(260.0),
                    );
                    if find.focus {
                        field.request_focus();
                        find.focus = false;
                    }
                    if field.changed() {
                        find.current = found.len().saturating_sub(1);
                        find.scroll = true;
                    }
                    let count = found.len();
                    let step = |find: &mut Find, forward: bool| {
                        if count > 0 {
                            find.current = if forward {
                                (find.current + 1) % count
                            } else {
                                (find.current + count - 1) % count
                            };
                            find.scroll = true;
                        }
                    };
                    if field.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                        step(find, false);
                        field.request_focus();
                    }
                    let place = if count == 0 {
                        if find.query.trim().is_empty() {
                            String::new()
                        } else {
                            "No matches".into()
                        }
                    } else {
                        format!("{} of {count}", count - find.current.min(count - 1))
                    };
                    ui.label(RichText::new(place).size(12.0).color(palette.secondary));
                    if ui
                        .small_button("↑")
                        .on_hover_text("Earlier match")
                        .clicked()
                    {
                        step(find, false);
                    }
                    if ui.small_button("↓").on_hover_text("Later match").clicked() {
                        step(find, true);
                    }
                    if ui.small_button("✕").on_hover_text("Close (Esc)").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.find = None;
        }
        found
    }

    fn pick(&mut self, key: &str, modifiers: Modifiers) {
        self.pane = Pane::Conversation;
        if modifiers.command {
            if self.picked.is_empty()
                && let Some(selected) = &self.selected
            {
                self.picked.insert(selected.clone());
            }
            if !self.picked.remove(key) {
                self.picked.insert(key.to_string());
            }
            self.anchor = Some(key.to_string());
        } else if modifiers.shift
            && let Some(anchor) = self.anchor.clone().or_else(|| self.selected.clone())
            && let (Some(from), Some(to)) = (
                self.shown.iter().position(|k| *k == anchor),
                self.shown.iter().position(|k| k == key),
            )
        {
            self.picked = self.shown[from.min(to)..=from.max(to)]
                .iter()
                .cloned()
                .collect();
        } else {
            self.picked.clear();
            self.anchor = Some(key.to_string());
        }
        self.selected = Some(key.to_string());
    }

    fn in_trash(&self) -> bool {
        let Scope::Mailbox { account, name } = &self.scope else {
            return false;
        };
        self.account(account).is_some_and(|account| {
            self.folders(&account)
                .iter()
                .any(|folder| &folder.name == name && folder.role == Some(Role::Trash))
        })
    }

    fn actions_menu(&self, ui: &mut Ui, targets: &[Conversation]) -> Option<Change> {
        let mut chosen = None;
        let unread = targets.iter().any(|c| c.unread > 0);
        let flagged = targets.iter().all(|c| c.flagged);
        let pinned = targets.iter().all(|c| c.pinned);
        let accounts: BTreeSet<&str> = targets.iter().map(|c| c.account.as_str()).collect();
        let account = (accounts.len() == 1)
            .then(|| accounts.first().and_then(|key| self.account(key)))
            .flatten();
        let items = [
            (
                if unread {
                    "Mark as Read"
                } else {
                    "Mark as Unread"
                },
                Change::Read(unread),
            ),
            (
                if flagged { "Unflag" } else { "Flag" },
                Change::Flag(!flagged),
            ),
            (if pinned { "Unpin" } else { "Pin" }, Change::Pin(!pinned)),
        ];
        for (label, change) in items {
            if ui.button(label).clicked() {
                chosen = Some(change);
                ui.close();
            }
        }
        ui.separator();
        if ui.button("Archive").clicked() {
            chosen = Some(Change::Archive);
            ui.close();
        }
        ui.add_enabled_ui(account.is_some(), |ui| {
            ui.menu_button("Move to", |ui| {
                for folder in account.iter().flat_map(|account| self.folders(account)) {
                    let here =
                        matches!(&self.scope, Scope::Mailbox { name, .. } if *name == folder.name);
                    let label = format!("{}{}", "    ".repeat(folder.depth), folder.label);
                    if folder.selectable && !here && ui.button(label).clicked() {
                        chosen = Some(Change::Move(folder.name.clone()));
                        ui.close();
                    }
                }
            });
        });
        if ui.button("Move to Junk").clicked() {
            chosen = Some(Change::Junk);
            ui.close();
        }
        let trash = if self.in_trash() {
            "Delete Forever…"
        } else {
            "Trash"
        };
        if ui.button(trash).clicked() {
            chosen = Some(Change::Trash);
            ui.close();
        }
        chosen
    }

    fn apply(&mut self, targets: Vec<Conversation>, change: Change) {
        if targets.is_empty() {
            return;
        }
        let change = if change == Change::Trash && self.in_trash() {
            Change::Delete
        } else {
            change
        };
        if change == Change::Delete {
            self.confirm_delete = Some(targets);
            return;
        }
        self.commit(targets, change);
    }

    fn commit(&mut self, targets: Vec<Conversation>, change: Change) {
        let leaves = matches!(
            change,
            Change::Archive | Change::Trash | Change::Junk | Change::Move(_) | Change::Delete
        );
        self.source.mail().change(&targets, &self.scope, change);
        if leaves {
            let gone: BTreeSet<&str> = targets.iter().map(|c| c.key.as_str()).collect();
            let at = self
                .selected
                .as_ref()
                .and_then(|key| self.shown.iter().position(|k| k == key));
            self.selected = at.and_then(|at| {
                self.shown[at..]
                    .iter()
                    .chain(self.shown[..at].iter().rev())
                    .find(|key| !gone.contains(key.as_str()))
                    .cloned()
            });
            self.picked.clear();
            self.reading = None;
        }
    }

    fn targets(&self, conversations: &[Conversation]) -> Vec<Conversation> {
        conversations
            .iter()
            .filter(|c| {
                if self.picked.len() > 1 {
                    self.picked.contains(&c.key)
                } else {
                    self.selected.as_ref() == Some(&c.key)
                }
            })
            .cloned()
            .collect()
    }

    fn keys(&mut self, ctx: &egui::Context, conversations: &[Conversation]) {
        if ctx.egui_wants_keyboard_input()
            || !matches!(self.pane, Pane::Conversation)
            || self.confirm_delete.is_some()
        {
            return;
        }
        let pressed = |modifiers, key| ctx.input_mut(|input| input.consume_key(modifiers, key));
        let change = if pressed(Modifiers::NONE, Key::Delete) {
            Some(Change::Trash)
        } else if pressed(Modifiers::NONE, Key::E) {
            Some(Change::Archive)
        } else if pressed(Modifiers::SHIFT, Key::U) {
            let targets = self.targets(conversations);
            Some(Change::Read(targets.iter().all(|c| c.unread > 0)))
        } else {
            None
        };
        if let Some(change) = change {
            let targets = self.targets(conversations);
            self.apply(targets, change);
        }
    }

    fn confirm_window(&mut self, ctx: &egui::Context) {
        let Some(targets) = &self.confirm_delete else {
            return;
        };
        let what = match targets.len() {
            1 => "this conversation".to_string(),
            count => format!("these {count} conversations"),
        };
        let mut answer = None;
        egui::Window::new("Delete Forever")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "Delete {what} from Trash for good? This cannot be undone."
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
        if ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)) {
            answer = Some(false);
        }
        match answer {
            Some(true) => {
                let targets = self.confirm_delete.take().unwrap_or_default();
                self.commit(targets, Change::Delete);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }

    fn several(&mut self, ui: &mut Ui, conversations: &[Conversation]) {
        let palette = self.palette;
        let targets = self.targets(conversations);
        let mut chosen = None;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(palette.canvas))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() / 2.0 - 60.0).max(24.0));
                    ui.label(
                        RichText::new(format!("{} conversations", targets.len()))
                            .size(18.0)
                            .color(palette.text),
                    );
                    ui.add_space(12.0);
                    let unread = targets.iter().any(|c| c.unread > 0);
                    let trash = if self.in_trash() {
                        "Delete Forever…"
                    } else {
                        "Trash"
                    };
                    let read = if unread {
                        "Mark as Read"
                    } else {
                        "Mark as Unread"
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(((ui.available_width() - 380.0) / 2.0).max(0.0));
                        if ui.button("Archive").clicked() {
                            chosen = Some(Change::Archive);
                        }
                        if ui.button(trash).clicked() {
                            chosen = Some(Change::Trash);
                        }
                        if ui.button(read).clicked() {
                            chosen = Some(Change::Read(unread));
                        }
                        ui.menu_button("More", |ui| {
                            if let Some(change) = self.actions_menu(ui, &targets) {
                                chosen = Some(change);
                            }
                        });
                    });
                    ui.add_space(8.0);
                    ui.label(
                        RichText::new("Ctrl-click to add or remove one, Shift-click for a range.")
                            .size(11.5)
                            .color(palette.secondary),
                    );
                });
            });
        if let Some(change) = chosen {
            self.apply(targets, change);
        }
    }

    fn folders(&self, account: &Account) -> Vec<FolderInfo> {
        let folders = self.source.mail().folders(&account.key());
        if folders.is_empty() {
            vec![FolderInfo {
                name: INBOX.into(),
                label: "Inbox".into(),
                depth: 0,
                role: Some(Role::Inbox),
                selectable: true,
                unread: 0,
            }]
        } else {
            folders
        }
    }

    fn scope_label(&self) -> String {
        match &self.scope {
            Scope::AllInboxes => "All Inboxes".into(),
            Scope::Mailbox { account, name } => self
                .account(account)
                .and_then(|account| {
                    self.folders(&account)
                        .into_iter()
                        .find(|folder| &folder.name == name)
                })
                .map_or_else(|| name.clone(), |folder| folder.label),
        }
    }

    fn switcher(&self, ui: &mut Ui) -> Option<Scope> {
        let palette = self.palette;
        let accounts = self.source.mail().accounts();
        let title = RichText::new(format!("{} ▾", self.scope_label()))
            .size(17.0)
            .strong()
            .color(palette.text);
        let mut chosen = None;
        let response = ui.menu_button(title, |ui| {
            ui.set_min_width(240.0);
            let count = |unread: u32| {
                if unread == 0 {
                    String::new()
                } else {
                    unread.to_string()
                }
            };
            let inboxes: u32 = accounts
                .iter()
                .filter_map(|account| {
                    self.folders(account)
                        .into_iter()
                        .find(|folder| folder.role == Some(Role::Inbox))
                })
                .map(|folder| folder.unread)
                .sum();
            if ui
                .add(
                    egui::Button::selectable(self.scope == Scope::AllInboxes, "All Inboxes")
                        .shortcut_text(count(inboxes)),
                )
                .clicked()
            {
                chosen = Some(Scope::AllInboxes);
                ui.close();
            }
            for account in &accounts {
                ui.separator();
                ui.label(
                    RichText::new(&account.address)
                        .size(11.5)
                        .color(palette.secondary),
                );
                for folder in self.folders(account) {
                    let scope = Scope::Mailbox {
                        account: account.key(),
                        name: folder.name.clone(),
                    };
                    let label = format!("{}{}", "    ".repeat(folder.depth), folder.label);
                    if !folder.selectable {
                        ui.label(RichText::new(label).color(palette.secondary));
                        continue;
                    }
                    if ui
                        .add(
                            egui::Button::selectable(self.scope == scope, label)
                                .shortcut_text(count(folder.unread)),
                        )
                        .clicked()
                    {
                        chosen = Some(scope);
                        ui.close();
                    }
                }
            }
        });
        response.response.on_hover_text("Choose a mailbox");
        chosen
    }

    fn status(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        ui.horizontal(|ui| {
            match self.source.mail().status() {
                Status::Syncing => {
                    ui.spinner();
                    ui.label(
                        RichText::new("Checking for mail…")
                            .size(12.0)
                            .color(palette.secondary),
                    );
                }
                Status::Synced(at) => {
                    ui.label(
                        RichText::new(format!("Updated {}", dates::time(at, self.clock)))
                            .size(12.0)
                            .color(palette.secondary),
                    );
                }
                Status::Failed(error) => {
                    let first = error.lines().next().unwrap_or_default().to_string();
                    ui.add(
                        egui::Label::new(RichText::new(first).size(12.0).color(palette.failure))
                            .truncate(),
                    )
                    .on_hover_text(error);
                }
                Status::NoAccounts => {}
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button("⟳")
                    .on_hover_text("Check for mail (Ctrl+R)")
                    .clicked()
                {
                    self.source.mail().refresh();
                }
                ui.toggle_value(&mut self.unread_only, "Unread")
                    .on_hover_text("Show only conversations with unread messages");
            });
        });
    }

    fn conversation_pane(&mut self, ui: &mut Ui, conversations: &[Conversation]) {
        let palette = self.palette;
        let found = self.selected.as_ref().and_then(|key| {
            conversations
                .iter()
                .find(|conversation| &conversation.key == key)
        });
        let Some(conversation) = found else {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(palette.canvas))
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        let text = if conversations.is_empty() {
                            "No conversations yet"
                        } else {
                            "Choose a conversation"
                        };
                        ui.label(RichText::new(text).size(15.0).color(palette.secondary));
                    });
                });
            return;
        };
        let conversation = conversation.clone();
        if conversation.unread > 0
            && ui
                .ctx()
                .input(|input| input.viewport().focused.unwrap_or(true))
            && self.marked.as_ref() != Some(&(conversation.key.clone(), conversation.unread))
        {
            self.marked = Some((conversation.key.clone(), conversation.unread));
            self.source.mail().mark_read(&conversation);
        }
        let account = self.account(&conversation.account);

        egui::Panel::top("conversation header")
            .frame(
                Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| {
                let actions = 190.0;
                let full = ui.available_rect_before_wrap();
                let mut chosen = None;
                let mut find = false;
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_max(
                            pos2(full.right() - actions, full.top()),
                            pos2(full.right(), full.top() + 36.0),
                        ))
                        .layout(Layout::right_to_left(Align::Center)),
                    |ui| {
                        ui.menu_button("…", |ui| {
                            if ui.button("Find in Conversation (Ctrl+Shift+F)").clicked() {
                                find = true;
                                ui.close();
                            }
                            ui.separator();
                            chosen = self.actions_menu(ui, std::slice::from_ref(&conversation));
                        })
                        .response
                        .on_hover_text("More actions");
                        let (flag, hint) = if conversation.flagged {
                            ("⚑", "Unflag")
                        } else {
                            ("⚐", "Flag")
                        };
                        let flag = RichText::new(flag).color(if conversation.flagged {
                            palette.flag
                        } else {
                            palette.text
                        });
                        if ui.button(flag).on_hover_text(hint).clicked() {
                            chosen = Some(Change::Flag(!conversation.flagged));
                        }
                        let hint = if self.in_trash() {
                            "Delete forever (Delete)"
                        } else {
                            "Move to Trash (Delete)"
                        };
                        if ui.button("🗑").on_hover_text(hint).clicked() {
                            chosen = Some(Change::Trash);
                        }
                        if ui.button("Archive").on_hover_text("Archive (E)").clicked() {
                            chosen = Some(Change::Archive);
                        }
                    },
                );
                if let Some(change) = chosen {
                    self.apply(vec![conversation.clone()], change);
                }
                if find {
                    self.open_find();
                }
                ui.vertical_centered(|ui| {
                    ui.set_max_width((full.width() - 2.0 * actions).max(120.0));
                    ui.add(
                        egui::Label::new(
                            RichText::new(list::title(&conversation))
                                .size(16.0)
                                .strong()
                                .color(palette.text),
                        )
                        .truncate(),
                    );
                    let members = conversation
                        .members
                        .iter()
                        .map(|person| person.display())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let members = if members.is_empty() {
                        "Only you".to_string()
                    } else {
                        members
                    };
                    ui.add(
                        egui::Label::new(
                            RichText::new(members).size(12.0).color(palette.secondary),
                        )
                        .truncate(),
                    );
                });
            });
        if self
            .find
            .as_ref()
            .is_some_and(|find| find.conversation != conversation.key)
        {
            self.find = None;
        }
        let found = self.find_bar(ui, &conversation);
        let current = self.find.as_ref().and_then(|find| {
            found
                .get(find.current.min(found.len().saturating_sub(1)))
                .copied()
        });
        let scroll = self.find.as_ref().is_some_and(|find| find.scroll);
        egui::Panel::bottom("composer")
            .frame(
                Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| self.composer(ui, &conversation, account.as_ref()));
        if self
            .reading
            .as_ref()
            .is_some_and(|reading| reading.conversation != conversation.key)
        {
            self.reading = None;
        }
        if self.auto_opened.as_ref() != Some(&conversation.key) {
            self.auto_opened = Some(conversation.key.clone());
            if self.settings.open_html
                && self.reading.is_none()
                && let Some(message) = conversation
                    .messages
                    .iter()
                    .rev()
                    .find(|message| message.html)
            {
                self.open_reading(&conversation, message.clone(), self.settings.load_images);
            }
        }
        if self.reading.is_some() {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(palette.canvas))
                .show(ui, |ui| self.reading_view(ui));
            return;
        }
        egui::CentralPanel::default()
            .frame(Frame::new().fill(palette.canvas))
            .show(ui, |ui| {
                let me = account
                    .as_ref()
                    .map_or(conversation.account.as_str(), |account| {
                        account.address.as_str()
                    });
                let action = ScrollArea::vertical()
                    .id_salt(&conversation.key)
                    .stick_to_bottom(true)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        bubbles::show(
                            ui,
                            &conversation,
                            me,
                            dates::today(),
                            self.clock,
                            &palette,
                            &bubbles::View {
                                images_by_default: self.settings.load_images,
                                found: &found,
                                current,
                                scroll,
                            },
                        )
                    })
                    .inner;
                if let Some(Action { message, choice }) = action {
                    match choice {
                        Choice::Email { images } => {
                            self.open_reading(&conversation, message, images)
                        }
                        choice => self.attachment(
                            &conversation.key,
                            &conversation.account,
                            &message,
                            choice,
                        ),
                    }
                }
            });
    }

    fn reading_view(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        let Some(reading) = &mut self.reading else {
            return;
        };
        if let Shown::Loading(reply) = &reading.shown
            && let Some(result) = arrived(reply, ui.ctx())
        {
            reading.shown = match result {
                Ok(html) => Shown::Ready {
                    page: Page::new(html.clone(), reading.images),
                    html,
                },
                Err(error) => Shown::Failed(error),
            };
        }
        let mut close = false;
        let mut toggle = false;
        let mut choice = None;
        Frame::new()
            .inner_margin(Margin::symmetric(16, 8))
            .show(ui, |ui| {
                let control = |text: &str| {
                    egui::Button::new(RichText::new(text).size(13.0).color(palette.accent))
                        .frame(false)
                };
                ui.horizontal(|ui| {
                    if ui
                        .add(control("☰ Chat View"))
                        .on_hover_text("Back to the conversation (Esc)")
                        .clicked()
                    {
                        close = true;
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (label, hint) = if reading.images {
                            ("👁 Hide Images", "Stop loading pictures from the sender's servers")
                        } else {
                            (
                                "👁 Show Images",
                                "Load pictures from the sender's servers, which can tell the sender you opened this",
                            )
                        };
                        if ui.add(control(label)).on_hover_text(hint).clicked() {
                            toggle = true;
                        }
                    });
                });
                if !reading.message.attachments.is_empty() {
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for (index, name) in reading.message.attachments.iter().enumerate() {
                            let chip = ui.add(bubbles::attachment_button(name, palette.text));
                            if let Some(chosen) = bubbles::attachment_choice(&chip, index) {
                                choice = Some(chosen);
                            }
                        }
                    });
                }
            });
        ui.separator();
        match &mut reading.shown {
            Shown::Loading(_) => {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new("Downloading the email…")
                            .size(12.0)
                            .color(palette.secondary),
                    );
                });
            }
            Shown::Failed(error) => {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format!("The email did not open: {error}"))
                            .color(palette.failure),
                    );
                });
            }
            Shown::Ready { page, .. } => {
                ScrollArea::vertical()
                    .id_salt(("email", &reading.conversation, reading.message.uid))
                    .auto_shrink(false)
                    .show(ui, |ui| page.show(ui));
            }
        }
        if toggle {
            reading.images = !reading.images;
            if let Shown::Ready { html, page } = &mut reading.shown {
                *page = Page::new(html.clone(), reading.images);
            }
        }
        if let Some(choice) = choice {
            let message = reading.message.clone();
            let account = reading.account.clone();
            let conversation = reading.conversation.clone();
            self.attachment(&conversation, &account, &message, choice);
        }
        if close {
            self.reading = None;
        }
    }

    fn open_reading(&mut self, conversation: &Conversation, message: Message, images: bool) {
        self.reading = Some(Reading {
            conversation: conversation.key.clone(),
            account: conversation.account.clone(),
            shown: Shown::Loading(self.source.mail().email(&conversation.account, &message)),
            message,
            images,
        });
    }

    fn attachment(&mut self, conversation: &str, account: &str, message: &Message, choice: Choice) {
        let mail = self.source.mail();
        let name = |index: usize| message.attachments.get(index).cloned().unwrap_or_default();
        let work = match choice {
            Choice::Email { .. } => return,
            Choice::Open(index) => Work::Open {
                name: name(index),
                reply: mail.attachment(account, message, index),
            },
            Choice::Save(index) => Work::Save {
                name: name(index),
                reply: mail.attachment(account, message, index),
            },
        };
        self.failure = None;
        self.fetching = Some(Fetching {
            conversation: conversation.to_string(),
            work,
        });
    }

    fn composer(&mut self, ui: &mut Ui, conversation: &Conversation, account: Option<&Account>) {
        let palette = self.palette;
        if let Some((key, error)) = &self.failure
            && key == &conversation.key
        {
            ui.label(RichText::new(error).size(12.0).color(palette.failure));
            ui.add_space(4.0);
        }
        if let Some(fetching) = &self.fetching
            && fetching.conversation == conversation.key
        {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(fetching.work.doing())
                        .size(12.0)
                        .color(palette.secondary),
                );
            });
            ui.add_space(4.0);
        }
        let sending = self
            .sending
            .as_ref()
            .is_some_and(|sending| sending.conversation == conversation.key);
        let editor_id = egui::Id::unique(("composer", &conversation.key));
        let focused = ui.ctx().memory(|memory| memory.has_focus(editor_id));
        let enter = focused && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
        let mut send = false;
        ui.horizontal(|ui| {
            let draft = self.drafts.entry(conversation.key.clone()).or_default();
            let ready = !draft.trim().is_empty() && !sending && account.is_some();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let button = egui::Button::new(RichText::new("Send").color(palette.on_accent))
                    .fill(palette.accent)
                    .corner_radius(14.0)
                    .min_size(egui::vec2(64.0, 30.0));
                if ui.add_enabled(ready, button).clicked() {
                    send = true;
                }
                if sending {
                    ui.spinner();
                }
                ui.add(
                    TextEdit::multiline(draft)
                        .id(editor_id)
                        .hint_text("Message")
                        .desired_rows(1)
                        .desired_width(ui.available_width())
                        .background_color(palette.field)
                        .margin(Margin::symmetric(10, 7))
                        .interactive(!sending),
                );
            });
            if enter && ready {
                send = true;
            }
        });
        ui.label(
            RichText::new("Enter sends, Shift+Enter starts a new line. Replies go to everyone in the conversation.")
                .size(10.5)
                .color(palette.secondary),
        );
        if send && let Some(account) = account {
            let body = self
                .drafts
                .get(&conversation.key)
                .cloned()
                .unwrap_or_default();
            let draft = zirbe_store::reply(conversation, account, body.trim().to_string());
            self.failure = None;
            self.sending = Some(Sending {
                conversation: conversation.key.clone(),
                reply: self.source.mail().send(draft),
            });
        }
    }

    fn new_pane(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        let accounts = self.source.mail().accounts();
        let Pane::New(new) = &mut self.pane else {
            return;
        };
        let mut done = false;
        if let Some(reply) = &new.pending {
            match reply.try_recv() {
                Ok(Ok(())) => done = true,
                Ok(Err(error)) => {
                    new.failure = Some(error);
                    new.pending = None;
                }
                Err(TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(150))
                }
                Err(TryRecvError::Disconnected) => new.pending = None,
            }
        }
        let mut cancel = false;
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::same(24)),
            )
            .show(ui, |ui| {
                ui.label(
                    RichText::new("New conversation")
                        .size(18.0)
                        .strong()
                        .color(palette.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "A conversation is named by its subject, which everyone in it sees.",
                    )
                    .size(12.0)
                    .color(palette.secondary),
                );
                ui.add_space(14.0);
                let width = ui.available_width().min(560.0);
                egui::Grid::new("new conversation")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        if accounts.len() > 1 {
                            ui.label("From");
                            egui::ComboBox::from_id_salt("from")
                                .selected_text(new.from.clone())
                                .show_ui(ui, |ui| {
                                    for account in &accounts {
                                        ui.selectable_value(
                                            &mut new.from,
                                            account.key(),
                                            account.address.clone(),
                                        );
                                    }
                                });
                            ui.end_row();
                        }
                        ui.label("To");
                        ui.add(
                            TextEdit::singleline(&mut new.to)
                                .hint_text("Names and addresses, separated by commas")
                                .desired_width(width - 80.0),
                        );
                        ui.end_row();
                        ui.label("Name");
                        ui.add(
                            TextEdit::singleline(&mut new.name)
                                .hint_text("What this conversation is about")
                                .desired_width(width - 80.0),
                        );
                        ui.end_row();
                    });
                ui.add_space(10.0);
                ui.add(
                    TextEdit::multiline(&mut new.body)
                        .hint_text("Message")
                        .desired_rows(8)
                        .desired_width(width)
                        .background_color(palette.field),
                );
                if let Some(failure) = &new.failure {
                    ui.add_space(6.0);
                    ui.label(RichText::new(failure).size(12.0).color(palette.failure));
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let ready = !new.to.trim().is_empty()
                        && !new.name.trim().is_empty()
                        && !new.body.trim().is_empty()
                        && new.pending.is_none();
                    let button = egui::Button::new(RichText::new("Send").color(palette.on_accent))
                        .fill(palette.accent);
                    if ui.add_enabled(ready, button).clicked() {
                        match zirbe_store::parse_people(&new.to) {
                            Ok(to) => {
                                new.failure = None;
                                new.pending = Some(self.source.mail().send(Draft {
                                    account: new.from.clone(),
                                    to,
                                    subject: new.name.trim().to_string(),
                                    body: new.body.trim().to_string(),
                                    ..Draft::default()
                                }));
                            }
                            Err(error) => new.failure = Some(error),
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if new.pending.is_some() {
                        ui.spinner();
                    }
                });
            });
        if done {
            let name = new.name.trim().to_string();
            self.selected = self
                .source
                .mail()
                .conversations()
                .iter()
                .find(|conversation| conversation.title == name)
                .map(|conversation| conversation.key.clone());
            self.pane = Pane::Conversation;
        } else if cancel {
            self.pane = Pane::Conversation;
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let palette = self.palette;
        let mut open = true;
        let mut sign_out = None;
        let mut settings = self.settings;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(RichText::new("Reading").strong().color(palette.text));
                ui.checkbox(&mut settings.open_html, "Open HTML email in Email View");
                ui.checkbox(&mut settings.load_images, "Load remote images");
                ui.checkbox(&mut settings.notify, "New mail notifications");
                ui.label(
                    RichText::new("Zirbe shows the plain-text version as a chat bubble by default. Email View shows the message as the sender laid it out. Loading remote images can let a sender know you opened the message and roughly where you are, so it stays off unless you turn it on.")
                        .size(11.0)
                        .color(palette.secondary),
                );
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new("Accounts").strong().color(palette.text));
                for account in self.source.mail().accounts() {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&account.address).color(palette.text));
                            ui.label(
                                RichText::new(format!(
                                    "{}:{}  ·  {}:{}",
                                    account.imap_host, account.imap_port, account.smtp_host, account.smtp_port
                                ))
                                .size(11.5)
                                .color(palette.secondary),
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if matches!(self.source, Source::Live(_)) && ui.button("Sign Out").clicked() {
                                sign_out = Some(account.clone());
                            }
                        });
                    });
                    ui.separator();
                }
                if let Source::Live(store) = &self.source {
                    if self.adding_account {
                        if let Outcome::SignedIn = self.sign_in.show(ui, store, &palette) {
                            self.adding_account = false;
                        }
                    } else if ui.button("Add Account").clicked() {
                        self.adding_account = true;
                    }
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new("Passwords stay in this computer's keyring. Mail is fetched only from your own servers.")
                        .size(11.0)
                        .color(palette.secondary),
                );
            });
        if let (Some(account), Source::Live(store)) = (sign_out, &self.source) {
            if matches!(&self.scope, Scope::Mailbox { account: key, .. } if *key == account.key()) {
                self.scope = Scope::AllInboxes;
                self.selected = None;
            }
            store.sign_out(&account);
        }
        if settings != self.settings {
            self.settings = settings;
            settings.save();
        }
        self.settings_open = open;
    }

    fn welcome(&mut self, ui: &mut Ui) {
        let palette = self.palette;
        egui::CentralPanel::default()
            .frame(Frame::new().fill(palette.canvas))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() / 2.0 - 220.0).max(24.0));
                    ui.label(RichText::new("Zirbe").size(26.0).strong().color(palette.text));
                    ui.add_space(8.0);
                    ui.set_max_width(460.0);
                    ui.label(
                        RichText::new("Email that reads like a conversation. Sign in with your email account; Zirbe talks only to your own mail servers.")
                            .color(palette.secondary),
                    );
                    ui.add_space(18.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width().min(460.0), 0.0),
                        Layout::top_down(Align::Min),
                        |ui| {
                            if let Source::Live(store) = &self.source {
                                self.sign_in.show(ui, store, &palette);
                            }
                        },
                    );
                });
            });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(feature = "screenshot")]
        if let Some(screenshot) = &mut self.screenshot {
            screenshot.tick(&ctx);
        }
        self.shortcuts(&ctx);
        self.poll_sending(&ctx);
        self.poll_fetching(&ctx);
        let needs_account = match &self.source {
            Source::Live(store) => {
                store.accounts().is_empty() && matches!(store.status(), Status::NoAccounts)
            }
            Source::Demo(_) => false,
        };
        if needs_account {
            self.welcome(ui);
            return;
        }
        let conversations = self.source.mail().conversations();
        self.update_matches(&conversations);
        self.news(&ctx, &conversations);
        self.keys(&ctx, &conversations);
        self.sidebar(ui, &conversations);
        match self.pane {
            Pane::Conversation if self.picked.len() > 1 => self.several(ui, &conversations),
            Pane::Conversation => self.conversation_pane(ui, &conversations),
            Pane::New(_) => self.new_pane(ui),
        }
        self.settings_window(&ctx);
        self.confirm_window(&ctx);
    }
}
