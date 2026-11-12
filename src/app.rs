// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use eframe::egui::containers::menu::{MenuButton, MenuConfig};
use eframe::egui::{
    self, Align, Frame, Key, Layout, Margin, Modifiers, PopupCloseBehavior, Rect, RichText,
    ScrollArea, TextEdit, Ui, UiBuilder, pos2,
};
use zirbe_store::{
    Account, Change, Conversation, Draft, FolderInfo, INBOX, MailSource, Message, Outgoing, Person,
    Query, Reply, Role, Scope, Status, Store,
};

use crate::bubbles::{self, Action, Choice};
use crate::composer::{self, Composer};
use crate::contacts::Contacts;
use crate::dates;
use crate::demo::Demo;
use crate::files;
use crate::filter::Filter;
use crate::list;
use crate::notify::Notifier;
use crate::page::Page;
use crate::palette::{self, Palette};
use crate::people;
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
    New(Box<Composer>),
}

#[derive(Default)]
struct Replying {
    target: Option<Message>,
    removed: Vec<Person>,
    attachments: Vec<Outgoing>,
    picking: Option<std::sync::mpsc::Receiver<Vec<Outgoing>>>,
}

enum Purpose {
    Forward { account: String, message: Message },
    Quoted { key: String },
    Draft { account: String, message: Message },
}

struct Reacting {
    conversation: String,
    account: String,
    target: Message,
    emoji: &'static str,
    at: std::time::Instant,
}

const COLUMN: f32 = 720.0;
const UNDO: Duration = Duration::from_secs(5);

struct Sending {
    conversation: String,
    draft: Draft,
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

struct Emptying {
    account: String,
    name: String,
    label: String,
}

enum ListAction {
    SelectAll,
    MarkAllRead,
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
    Original {
        purpose: Box<Purpose>,
        reply: Reply<zirbe_store::Original>,
    },
    Open {
        name: String,
        reply: Reply<Vec<u8>>,
    },
    Save {
        name: String,
        reply: Reply<Vec<u8>>,
    },
    Saving {
        name: String,
        reply: Reply,
    },
}

impl Work {
    fn doing(&self) -> String {
        match self {
            Work::Original { purpose, .. } => match purpose.as_ref() {
                Purpose::Forward { .. } => "Preparing the forward…".into(),
                Purpose::Quoted { .. } => "Downloading the quoted text…".into(),
                Purpose::Draft { .. } => "Opening the draft…".into(),
            },
            Work::Open { name, .. } | Work::Save { name, .. } => format!("Downloading {name}…"),
            Work::Saving { name, .. } => format!("Saving {name}…"),
        }
    }

    fn failed(&self, error: &str) -> String {
        match self {
            Work::Original { .. } => format!("The message could not be downloaded: {error}"),
            Work::Open { name, .. } => format!("{name} did not open: {error}"),
            Work::Save { name, .. } | Work::Saving { name, .. } => {
                format!("{name} was not saved: {error}")
            }
        }
    }
}

fn column<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let full = ui.available_rect_before_wrap();
    let width = full.width().min(COLUMN);
    let mut rect = full;
    rect.min.x = full.center().x - width / 2.0;
    rect.max.x = rect.min.x + width;
    ui.scope_builder(UiBuilder::new().max_rect(rect).layout(*ui.layout()), add)
        .inner
}

fn only_from(conversation: &Conversation, sender: &Person) -> bool {
    let mut others = conversation
        .messages
        .iter()
        .filter_map(|message| message.from.as_ref())
        .filter(|from| !from.is(&conversation.account))
        .peekable();
    others.peek().is_some() && others.all(|from| from.is(&sender.address))
}

fn ask(ctx: &egui::Context, title: &str, text: &str, yes: &str) -> Option<bool> {
    let mut answer = None;
    egui::Window::new(title)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label(text);
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(yes).clicked() {
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
    answer
}

pub(crate) fn arrived<T>(reply: &Reply<T>, ctx: &egui::Context) -> Option<Result<T, String>> {
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
    filter: Filter,
    reveal: bool,
    confirm_delete: Option<Vec<Conversation>>,
    confirm_empty: Option<Emptying>,
    confirm_sweep: Option<(Person, Vec<Conversation>)>,
    notifier: Option<Notifier>,
    query: String,
    matches: Option<(String, usize, BTreeSet<String>)>,
    server_search: Option<ServerSearch>,
    find: Option<Find>,
    replying: HashMap<String, Replying>,
    retry: Option<(String, Draft)>,
    expanded: HashMap<String, String>,
    opening_draft: Option<String>,
    known: Option<(usize, Vec<Person>)>,
    contacts: Contacts,
    labels: HashMap<String, String>,
    aliases: HashMap<String, String>,
    reauth: HashMap<String, (String, Option<Reply>, Option<String>)>,
    confirm_block: Option<(String, Person)>,
    thumbs: HashMap<String, bubbles::Thumb>,
    thumb_loads: Vec<(String, Reply<Vec<u8>>)>,
    reacting: Option<Reacting>,
    reaction_sends: Vec<(String, Reply)>,
    auto_opened: Option<String>,
    adding_account: bool,
    sign_in: SignIn,
    marked: Option<(String, usize)>,
    #[cfg(feature = "screenshot")]
    screenshot: Option<crate::screenshot::Screenshot>,
    #[cfg(feature = "screenshot")]
    screenshot_opening: Option<String>,
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
            filter: Filter::default(),
            reveal: false,
            confirm_delete: None,
            confirm_empty: None,
            confirm_sweep: None,
            notifier: None,
            query: String::new(),
            matches: None,
            server_search: None,
            find: None,
            replying: HashMap::new(),
            retry: None,
            expanded: HashMap::new(),
            opening_draft: None,
            known: None,
            contacts: Contacts::default(),
            labels: HashMap::new(),
            aliases: HashMap::new(),
            reauth: HashMap::new(),
            confirm_block: None,
            thumbs: HashMap::new(),
            thumb_loads: Vec::new(),
            reacting: None,
            reaction_sends: Vec::new(),
            auto_opened: None,
            adding_account: false,
            sign_in: SignIn::default(),
            marked: None,
            #[cfg(feature = "screenshot")]
            screenshot: crate::screenshot::Screenshot::from_environment(),
            #[cfg(feature = "screenshot")]
            screenshot_opening: None,
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
                if self.selected.is_none() {
                    self.screenshot_opening = Some(title.to_string());
                }
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
        self.pane = Pane::New(Box::new(Composer::new(from)));
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
        if let Pane::New(composer) = &mut self.pane
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
            && composer.cancel()
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
                self.replying.remove(&sending.conversation);
                self.retry = None;
                self.sending = None;
            }
            Ok(Err(error)) => {
                self.failure = Some((sending.conversation.clone(), format!("Not sent. {error}")));
                self.retry = Some((sending.conversation.clone(), sending.draft.clone()));
                self.sending = None;
            }
            Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(150)),
            Err(TryRecvError::Disconnected) => self.sending = None,
        }
    }

    fn poll_thumbs(&mut self, ctx: &egui::Context) {
        let mut waiting = Vec::new();
        for (key, reply) in std::mem::take(&mut self.thumb_loads) {
            match arrived(&reply, ctx) {
                None => waiting.push((key, reply)),
                Some(result) => {
                    let thumb = result
                        .ok()
                        .and_then(|bytes| image::load_from_memory(&bytes).ok())
                        .map(|image| {
                            let small = image.thumbnail(480, 480).to_rgba8();
                            let size = [small.width() as usize, small.height() as usize];
                            let pixels =
                                egui::ColorImage::from_rgba_unmultiplied(size, small.as_raw());
                            bubbles::Thumb::Ready(ctx.load_texture(
                                &key,
                                pixels,
                                egui::TextureOptions::LINEAR,
                            ))
                        })
                        .unwrap_or(bubbles::Thumb::Failed);
                    self.thumbs.insert(key, thumb);
                }
            }
        }
        self.thumb_loads = waiting;
    }

    fn flush_reaction(&mut self) {
        let Some(reacting) = self.reacting.take() else {
            return;
        };
        let conversations = self.source.mail().conversations();
        let (Some(conversation), Some(account)) = (
            conversations
                .iter()
                .find(|c| c.key == reacting.conversation),
            self.account(&reacting.account),
        ) else {
            return;
        };
        let draft = zirbe_store::reaction(conversation, &account, &reacting.target, reacting.emoji);
        self.reaction_sends
            .push((reacting.conversation, self.source.mail().send(draft)));
    }

    fn poll_reactions(&mut self, ctx: &egui::Context) {
        if let Some(reacting) = &self.reacting {
            let left = UNDO.saturating_sub(reacting.at.elapsed());
            if left.is_zero() {
                self.flush_reaction();
            } else {
                ctx.request_repaint_after(left.min(Duration::from_millis(250)));
            }
        }
        let mut waiting = Vec::new();
        for (key, reply) in std::mem::take(&mut self.reaction_sends) {
            match arrived(&reply, ctx) {
                None => waiting.push((key, reply)),
                Some(Ok(())) => {}
                Some(Err(error)) => {
                    self.failure = Some((key, format!("The reaction was not sent: {error}")));
                }
            }
        }
        self.reaction_sends = waiting;
    }

    fn poll_fetching(&mut self, ctx: &egui::Context) {
        let Some(Fetching { conversation, work }) = self.fetching.take() else {
            return;
        };
        let failed = |error: String| work.failed(&error);
        let mut original = None;
        let next = match &work {
            Work::Original { reply, .. } => arrived(reply, ctx).map(|result| {
                result.map(|found| {
                    original = Some(found);
                    None
                })
            }),
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
        let error = match next {
            None => {
                self.fetching = Some(Fetching { conversation, work });
                return;
            }
            Some(Ok(Some(work))) => {
                self.fetching = Some(Fetching { conversation, work });
                return;
            }
            Some(Ok(None)) => None,
            Some(Err(error)) => Some(failed(error)),
        };
        if let Some(error) = error {
            self.failure = Some((conversation, error));
            return;
        }
        if let (Work::Original { purpose, .. }, Some(original)) = (work, original) {
            match *purpose {
                Purpose::Forward { account, message } => {
                    self.reading = None;
                    self.pane = Pane::New(Box::new(Composer::forward(account, &message, original)));
                }
                Purpose::Quoted { key } => {
                    self.expanded.insert(key, original.text);
                }
                Purpose::Draft { account, message } => {
                    let place = (message.mailbox.clone(), message.uid);
                    self.pane = Pane::New(Box::new(Composer::draft(
                        account, &message, original, place,
                    )));
                }
            }
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
                let mut action = None;
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
                        ui.menu_button("⋯", |ui| {
                            let select_all =
                                egui::Button::new("Select All").shortcut_text("Ctrl+A");
                            if ui.add(select_all).clicked() {
                                action = Some(ListAction::SelectAll);
                                ui.close();
                            }
                            if ui.button("Mark All as Read").clicked() {
                                action = Some(ListAction::MarkAllRead);
                                ui.close();
                            }
                        })
                        .response
                        .on_hover_text("List actions");
                    });
                });
                ui.add_space(6.0);
                self.search_field(ui);
                ui.add_space(4.0);
                if let Some(emptying) = self.emptiable()
                    && conversations.iter().any(|c| self.scope.holds(c))
                {
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);
                        if ui
                            .small_button(format!("Empty {}…", emptying.label))
                            .clicked()
                        {
                            self.confirm_empty = Some(emptying);
                        }
                    });
                    ui.add_space(4.0);
                }
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
                        self.filter.admits(conversation)
                            || self.selected.as_ref() == Some(&conversation.key)
                            || self.picked.contains(&conversation.key)
                    })
                    .collect();
                self.shown = shown.iter().map(|c| c.key.clone()).collect();
                match action {
                    Some(ListAction::SelectAll) => self.select_all(),
                    Some(ListAction::MarkAllRead) => {
                        let unread = shown
                            .iter()
                            .filter(|c| c.unread > 0)
                            .map(|c| (*c).clone())
                            .collect();
                        self.apply(unread, Change::Read(true));
                    }
                    None => {}
                }
                if shown.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        let text = if self.matches.is_some() {
                            "No matches here"
                        } else if self.filter.only_unread() {
                            "You're all caught up"
                        } else if self.filter.active() {
                            "Nothing here matches the filter"
                        } else {
                            "No conversations here"
                        };
                        ui.label(RichText::new(text).size(13.0).color(palette.secondary));
                    });
                }
                let mut pending = None;
                let accounts = self.source.mail().accounts();
                let colours: HashMap<String, egui::Color32> = accounts
                    .iter()
                    .map(|account| (account.key(), palette::account(account.colour)))
                    .collect();
                let several = accounts.len() > 1;
                ScrollArea::vertical().auto_shrink(false).show_rows(
                    ui,
                    list::height(self.settings.preview),
                    shown.len(),
                    |ui, range| {
                        if std::mem::take(&mut self.reveal)
                            && let Some(at) = self
                                .selected
                                .as_ref()
                                .and_then(|key| shown.iter().position(|c| &c.key == key))
                        {
                            let row =
                                list::height(self.settings.preview) + ui.spacing().item_spacing.y;
                            let top = ui.cursor().top() + (at as f32 - range.start as f32) * row;
                            let rect = Rect::from_min_size(
                                pos2(ui.cursor().left(), top),
                                egui::vec2(1.0, row),
                            );
                            ui.scroll_to_rect(rect, None);
                        }
                        let today = dates::today();
                        for conversation in &shown[range] {
                            let key = &conversation.key;
                            let selected = matches!(self.pane, Pane::Conversation)
                                && (self.selected.as_ref() == Some(key)
                                    || self.picked.contains(key));
                            let look = list::Look {
                                lines: self.settings.preview,
                                dot: several
                                    .then(|| colours.get(&conversation.account).copied())
                                    .flatten(),
                                faces: self.contacts.faces(),
                            };
                            let response = list::row(
                                ui,
                                conversation,
                                selected,
                                today,
                                self.clock,
                                &palette,
                                &look,
                            );
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

    fn select_all(&mut self) {
        match self.shown.as_slice() {
            [] => {}
            [only] => self.pick(&only.clone(), Modifiers::NONE),
            all => {
                self.picked = all.iter().cloned().collect();
                if self
                    .selected
                    .as_ref()
                    .is_none_or(|key| !self.picked.contains(key))
                {
                    self.selected = all.first().cloned();
                }
                self.pane = Pane::Conversation;
            }
        }
    }

    fn step(&mut self, by: isize, modifiers: Modifiers) {
        let at = self
            .selected
            .as_ref()
            .and_then(|key| self.shown.iter().position(|k| k == key));
        let next = match at {
            Some(at) => at
                .checked_add_signed(by)
                .filter(|next| *next < self.shown.len()),
            None => (!self.shown.is_empty()).then_some(0),
        };
        if let Some(key) = next.map(|next| self.shown[next].clone()) {
            self.pick(&key, modifiers);
            self.reveal = true;
        }
    }

    fn emptiable(&self) -> Option<Emptying> {
        let Scope::Mailbox { account, name } = &self.scope else {
            return None;
        };
        self.folders(&self.account(account)?)
            .into_iter()
            .find(|folder| {
                &folder.name == name && matches!(folder.role, Some(Role::Trash | Role::Junk))
            })
            .map(|folder| Emptying {
                account: account.clone(),
                name: name.clone(),
                label: folder.label,
            })
    }

    fn in_trash(&self) -> bool {
        self.in_role(Role::Trash)
    }

    fn in_role(&self, role: Role) -> bool {
        let Scope::Mailbox { account, name } = &self.scope else {
            return false;
        };
        self.account(account).is_some_and(|account| {
            self.folders(&account)
                .iter()
                .any(|folder| &folder.name == name && folder.role == Some(role))
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
        let leaves = change.leaves();
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
            || self.confirm_block.is_some()
            || self.confirm_empty.is_some()
            || self.confirm_sweep.is_some()
        {
            return;
        }
        let pressed = |modifiers, key| ctx.input_mut(|input| input.consume_key(modifiers, key));
        if pressed(Modifiers::COMMAND, Key::A) {
            self.select_all();
            return;
        }
        for (key, by) in [(Key::ArrowDown, 1), (Key::ArrowUp, -1)] {
            for modifiers in [Modifiers::SHIFT, Modifiers::NONE] {
                if pressed(modifiers, key) {
                    self.step(by, modifiers);
                    return;
                }
            }
        }
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
        let text = format!("Delete {what} from Trash for good? This cannot be undone.");
        match ask(ctx, "Delete Forever", &text, "Delete") {
            Some(true) => {
                let targets = self.confirm_delete.take().unwrap_or_default();
                self.commit(targets, Change::Delete);
            }
            Some(false) => self.confirm_delete = None,
            None => {}
        }
    }

    fn empty_window(&mut self, ctx: &egui::Context) {
        let Some(emptying) = &self.confirm_empty else {
            return;
        };
        let accounts = self.source.mail().accounts();
        let place = match accounts.iter().find(|a| a.key() == emptying.account) {
            Some(account) if accounts.len() > 1 => {
                format!("{} for {}", emptying.label, account.title())
            }
            _ => emptying.label.clone(),
        };
        let text = format!(
            "Delete everything in {place} for good? Mail that arrived after Zirbe last checked is kept. This cannot be undone."
        );
        let title = format!("Empty {}", emptying.label);
        match ask(ctx, &title, &text, "Empty") {
            Some(true) => {
                if let Some(emptying) = self.confirm_empty.take() {
                    self.source.mail().empty(&emptying.account, &emptying.name);
                    self.selected = None;
                    self.picked.clear();
                    self.reading = None;
                }
            }
            Some(false) => self.confirm_empty = None,
            None => {}
        }
    }

    fn sweep_window(&mut self, ctx: &egui::Context) {
        let Some((sender, targets)) = &self.confirm_sweep else {
            return;
        };
        let what = match targets.len() {
            1 => "the conversation".to_string(),
            count => format!("all {count} conversations"),
        };
        let text = format!(
            "Move {what} with {} <{}> from {} to Trash?",
            sender.display(),
            sender.address,
            self.scope_label()
        );
        match ask(ctx, "Trash All from Sender", &text, "Trash") {
            Some(true) => {
                if let Some((_, targets)) = self.confirm_sweep.take() {
                    self.commit(targets, Change::Trash);
                }
            }
            Some(false) => self.confirm_sweep = None,
            None => {}
        }
    }

    fn block_window(&mut self, ctx: &egui::Context) {
        let Some((account, sender)) = &self.confirm_block else {
            return;
        };
        let text = format!(
            "Block {} <{}>? Their mail moves to Junk now, and so does anything new from them. Settings can unblock them.",
            sender.display(),
            sender.address
        );
        match ask(ctx, "Block Sender", &text, "Block") {
            Some(true) => {
                self.source.mail().block(account, &sender.address, true);
                self.confirm_block = None;
            }
            Some(false) => self.confirm_block = None,
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
                ui.horizontal(|ui| {
                    let (spot, _) =
                        ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        spot.center(),
                        4.0,
                        palette::account(account.colour),
                    );
                    ui.label(
                        RichText::new(account.title())
                            .size(11.5)
                            .color(palette.secondary),
                    );
                });
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
                let filter = self.filter.summary();
                let button = egui::Button::new("Filter").selected(self.filter.active());
                MenuButton::from_button(button)
                    .config(
                        MenuConfig::new().close_behavior(PopupCloseBehavior::CloseOnClickOutside),
                    )
                    .ui(ui, |ui| self.filter.menu(ui, &palette))
                    .0
                    .on_hover_text(filter);
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
        if self.in_role(Role::Drafts)
            && let Scope::Mailbox { name, .. } = &self.scope
            && let Some(draft) = conversation
                .messages
                .iter()
                .rev()
                .find(|message| &message.mailbox == name && message.uid != 0)
        {
            if self.opening_draft.as_ref() != Some(&conversation.key) {
                self.opening_draft = Some(conversation.key.clone());
                self.fetching = Some(Fetching {
                    conversation: conversation.key.clone(),
                    work: Work::Original {
                        reply: self.source.mail().original(&conversation.account, draft),
                        purpose: Box::new(Purpose::Draft {
                            account: conversation.account.clone(),
                            message: draft.clone(),
                        }),
                    },
                });
            }
            egui::CentralPanel::default()
                .frame(Frame::new().fill(palette.canvas))
                .show(ui, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.spinner();
                    });
                });
            return;
        }
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
                let mut block = None;
                let mut sweep = None;
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
                            if let Some(sender) = conversation
                                .messages
                                .iter()
                                .rev()
                                .filter_map(|message| message.from.as_ref())
                                .find(|from| !from.is(&conversation.account))
                            {
                                if ui.button(format!("Block {}…", sender.display())).clicked() {
                                    block = Some(sender.clone());
                                    ui.close();
                                }
                                if !self.in_trash()
                                    && ui
                                        .button(format!("Trash All from {}…", sender.display()))
                                        .clicked()
                                {
                                    sweep = Some(sender.clone());
                                    ui.close();
                                }
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
                if let Some(sender) = sweep {
                    let targets = conversations
                        .iter()
                        .filter(|c| c.account == conversation.account && self.scope.holds(c))
                        .filter(|c| only_from(c, &sender))
                        .cloned()
                        .collect();
                    self.confirm_sweep = Some((sender, targets));
                }
                if let Some(sender) = block {
                    self.confirm_block = Some((conversation.account.clone(), sender));
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
            .show(ui, |ui| {
                column(ui, |ui| self.composer(ui, &conversation, account.as_ref()))
            });
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
                let me = account.clone().unwrap_or_else(|| Account {
                    address: conversation.account.clone(),
                    ..Account::default()
                });
                let wanted = std::cell::RefCell::new(Vec::new());
                let action = ScrollArea::vertical()
                    .id_salt(&conversation.key)
                    .stick_to_bottom(true)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        column(ui, |ui| {
                            bubbles::show(
                                ui,
                                &conversation,
                                &me,
                                dates::today(),
                                self.clock,
                                &palette,
                                &bubbles::View {
                                    images_by_default: self.settings.load_images,
                                    expanded: &self.expanded,
                                    thumbs: &self.thumbs,
                                    wanted: &wanted,
                                    pending: self
                                        .reacting
                                        .as_ref()
                                        .filter(|reacting| {
                                            reacting.conversation == conversation.key
                                        })
                                        .and_then(|reacting| {
                                            reacting
                                                .target
                                                .message_id
                                                .as_deref()
                                                .map(|id| (id, reacting.emoji))
                                        }),
                                    clock: self.clock,
                                    found: &found,
                                    current,
                                    scroll,
                                    faces: self.contacts.faces(),
                                },
                            )
                        })
                    })
                    .inner;
                for (message, index) in wanted.into_inner() {
                    let key = bubbles::thumb_key(&message, index);
                    if !self.thumbs.contains_key(&key) {
                        self.thumbs.insert(key.clone(), bubbles::Thumb::Loading);
                        self.thumb_loads.push((
                            key,
                            self.source
                                .mail()
                                .attachment(&conversation.account, &message, index),
                        ));
                    }
                }
                if let Some(Action { message, choice }) = action {
                    match choice {
                        Choice::React(emoji) => {
                            self.flush_reaction();
                            self.reacting = Some(Reacting {
                                conversation: conversation.key.clone(),
                                account: conversation.account.clone(),
                                target: message,
                                emoji,
                                at: std::time::Instant::now(),
                            });
                        }
                        Choice::Join(link) => {
                            let _ = open::that_detached(link);
                        }
                        Choice::AddToCalendar => {
                            if let Some(invite) = &message.invite
                                && let Err(error) =
                                    files::open("invitation.ics", invite.ics.as_bytes())
                            {
                                self.failure = Some((
                                    conversation.key.clone(),
                                    format!("The invitation did not open: {error}"),
                                ));
                            }
                        }
                        Choice::Web { images } => self.open_reading(&conversation, message, images),
                        Choice::ReplyTo => {
                            self.replying
                                .entry(conversation.key.clone())
                                .or_default()
                                .target = Some(message);
                            let editor = egui::Id::unique(("composer", &conversation.key));
                            ui.memory_mut(|memory| memory.request_focus(editor));
                        }
                        Choice::Copy => {
                            let key = bubbles::key_of(&message);
                            let text = self.expanded.get(&key).cloned().unwrap_or(message.text);
                            ui.ctx().copy_text(text);
                        }
                        Choice::Quoted => {
                            let key = bubbles::key_of(&message);
                            if self.expanded.remove(&key).is_none() {
                                self.fetching = Some(Fetching {
                                    conversation: conversation.key.clone(),
                                    work: Work::Original {
                                        reply: self
                                            .source
                                            .mail()
                                            .original(&conversation.account, &message),
                                        purpose: Box::new(Purpose::Quoted { key }),
                                    },
                                });
                            }
                        }
                        Choice::Forward => {
                            self.fetching = Some(Fetching {
                                conversation: conversation.key.clone(),
                                work: Work::Original {
                                    reply: self
                                        .source
                                        .mail()
                                        .original(&conversation.account, &message),
                                    purpose: Box::new(Purpose::Forward {
                                        account: conversation.account.clone(),
                                        message,
                                    }),
                                },
                            });
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
            Choice::Web { .. }
            | Choice::React(_)
            | Choice::Join(_)
            | Choice::AddToCalendar
            | Choice::ReplyTo
            | Choice::Forward
            | Choice::Copy
            | Choice::Quoted => return,
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
        let key = conversation.key.clone();
        if let Some((failed, error)) = &self.failure
            && *failed == key
        {
            let mut retry = false;
            ui.horizontal(|ui| {
                ui.label(RichText::new(error).size(12.0).color(palette.failure));
                if self.retry.as_ref().is_some_and(|(at, _)| *at == key)
                    && self.sending.is_none()
                    && ui.small_button("Retry").clicked()
                {
                    retry = true;
                }
            });
            if retry && let Some((_, draft)) = self.retry.take() {
                self.failure = None;
                self.sending = Some(Sending {
                    conversation: key.clone(),
                    reply: self.source.mail().send(draft.clone()),
                    draft,
                });
            }
            ui.add_space(4.0);
        }
        if let Some(fetching) = &self.fetching
            && fetching.conversation == key
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
        if let Some(reacting) = &self.reacting
            && reacting.conversation == key
        {
            let mut undo = false;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Reacted {}", reacting.emoji))
                        .size(12.0)
                        .color(palette.secondary),
                );
                let link =
                    egui::Button::new(RichText::new("Undo").size(12.0).color(palette.accent))
                        .frame(false);
                undo = ui.add(link).clicked();
            });
            if undo {
                self.reacting = None;
            }
            ui.add_space(4.0);
        }
        let sending = self
            .sending
            .as_ref()
            .is_some_and(|sending| sending.conversation == key);
        let state = self.replying.entry(key.clone()).or_default();
        if let Some(picking) = &state.picking {
            match picking.try_recv() {
                Ok(files) => {
                    state.attachments.extend(files);
                    state.picking = None;
                }
                Err(TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(200))
                }
                Err(TryRecvError::Disconnected) => state.picking = None,
            }
        }
        if let Some(target) = &state.target {
            let mut cancel = false;
            ui.horizontal(|ui| {
                let sender = target.from.as_ref().map_or("someone", |from| from.short());
                let snippet: String = target
                    .text
                    .split_whitespace()
                    .take(12)
                    .collect::<Vec<_>>()
                    .join(" ");
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Replying to {sender}: {snippet}"))
                            .size(12.0)
                            .color(palette.secondary),
                    )
                    .truncate(),
                );
                cancel = ui
                    .small_button("✕")
                    .on_hover_text("Reply to the latest message instead")
                    .clicked();
            });
            if cancel {
                state.target = None;
            }
        }
        let recipients: Vec<Person> = account
            .map(|account| {
                let draft =
                    zirbe_store::reply(conversation, account, String::new(), state.target.as_ref());
                draft.to.into_iter().chain(draft.cc).collect()
            })
            .unwrap_or_default();
        let mut remove = None;
        let mut undo = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("To").size(12.0).color(palette.secondary));
            for person in recipients
                .iter()
                .filter(|person| !state.removed.iter().any(|gone| gone.is(&person.address)))
            {
                let chip = egui::Button::new(
                    RichText::new(format!("{} ✕", person.short()))
                        .size(12.0)
                        .color(palette.text),
                )
                .fill(palette.field)
                .corner_radius(10.0);
                if ui
                    .add(chip)
                    .on_hover_text(format!("Leave {} out of this reply", person.address))
                    .clicked()
                {
                    remove = Some(person.clone());
                }
            }
            if let Some(last) = state.removed.last() {
                let link = egui::Button::new(
                    RichText::new(format!("Undo removing {}", last.short()))
                        .size(12.0)
                        .color(palette.accent),
                )
                .frame(false);
                undo = ui.add(link).clicked();
            }
        });
        if let Some(person) = remove {
            state.removed.push(person);
        }
        if undo {
            state.removed.pop();
        }
        let mut drop_file = None;
        if !state.attachments.is_empty() {
            ui.horizontal_wrapped(|ui| {
                for (index, file) in state.attachments.iter().enumerate() {
                    if ui
                        .small_button(format!("📎 {} ✕", file.name))
                        .on_hover_text("Remove")
                        .clicked()
                    {
                        drop_file = Some(index);
                    }
                }
            });
            if files::size(&state.attachments) > files::LARGE {
                ui.label(
                    RichText::new(
                        "These attachments come to more than 25 MB; many mail servers refuse that.",
                    )
                    .size(12.0)
                    .color(palette.failure),
                );
            }
        }
        if let Some(index) = drop_file {
            state.attachments.remove(index);
        }
        let editor_id = egui::Id::unique(("composer", &key));
        let focused = ui.ctx().memory(|memory| memory.has_focus(editor_id));
        let enter = focused
            && ui.input_mut(|input| {
                let before = input.events.len();
                input.events.retain(|event| {
                    !matches!(
                        event,
                        egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. }
                            if modifiers.is_none()
                    )
                });
                input.events.len() != before
            });
        let mut send = false;
        let mut pick = false;
        ui.horizontal(|ui| {
            let draft = self.drafts.entry(key.clone()).or_default();
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
                pick = ui
                    .button("📎")
                    .on_hover_text("Attach files, or drop them on the window")
                    .clicked();
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
            RichText::new("Enter sends, Shift+Enter starts a new line.")
                .size(10.5)
                .color(palette.secondary),
        );
        let state = self.replying.entry(key.clone()).or_default();
        if pick && state.picking.is_none() {
            state.picking = Some(files::pick());
        }
        if send && let Some(account) = account {
            let body = self.drafts.get(&key).cloned().unwrap_or_default();
            let mut draft = zirbe_store::reply(
                conversation,
                account,
                body.trim().to_string(),
                state.target.as_ref(),
            );
            let kept = |people: Vec<Person>| -> Vec<Person> {
                people
                    .into_iter()
                    .filter(|person| !state.removed.iter().any(|gone| gone.is(&person.address)))
                    .collect()
            };
            draft.to = kept(draft.to);
            draft.cc = kept(draft.cc);
            draft.attachments = state.attachments.clone();
            if draft.to.is_empty() && draft.cc.is_empty() {
                self.failure = Some((
                    key,
                    "Not sent. Everyone has been left out of this reply.".into(),
                ));
                return;
            }
            self.failure = None;
            self.retry = None;
            self.sending = Some(Sending {
                conversation: key,
                reply: self.source.mail().send(draft.clone()),
                draft,
            });
        }
    }

    fn new_pane(&mut self, ui: &mut Ui, conversations: &Arc<Vec<Conversation>>) {
        let accounts = self.source.mail().accounts();
        let source = Arc::as_ptr(conversations) as usize;
        if self.known.as_ref().is_none_or(|(from, _)| *from != source) {
            let known = people::known(conversations, self.contacts.people(), &accounts);
            self.known = Some((source, known));
        }
        let known = self
            .known
            .as_ref()
            .map_or(&[][..], |(_, people)| people.as_slice());
        let palette = self.palette;
        let Pane::New(composer) = &mut self.pane else {
            return;
        };
        match composer.show(ui, self.source.mail(), &accounts, known, &palette) {
            composer::Outcome::Open => {}
            composer::Outcome::Closed => self.pane = Pane::Conversation,
            composer::Outcome::Sent { subject } => {
                let fresh = self.source.mail().conversations();
                self.selected = fresh
                    .iter()
                    .filter(|c| subject.is_empty() || c.title == subject)
                    .filter(|c| c.messages.iter().any(|m| m.uid == 0))
                    .max_by_key(|c| c.latest())
                    .map(|c| c.key.clone());
                self.pane = Pane::Conversation;
            }
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
                ui.checkbox(&mut settings.open_html, "Open HTML email in Web View");
                ui.checkbox(&mut settings.load_images, "Load remote images");
                ui.checkbox(&mut settings.notify, "New mail notifications");
                ui.horizontal(|ui| {
                    ui.label("Preview");
                    let names = ["None", "1 line", "2 lines", "3 lines"];
                    egui::ComboBox::from_id_salt("preview")
                        .selected_text(names[usize::from(settings.preview.min(3))])
                        .show_ui(ui, |ui| {
                            for (lines, name) in names.iter().enumerate() {
                                ui.selectable_value(&mut settings.preview, lines as u8, *name);
                            }
                        });
                });
                ui.label(
                    RichText::new("Zirbe shows the plain-text version as a chat bubble by default. Web View shows the message as the sender laid it out. Loading remote images can let a sender know you opened the message and roughly where you are, so it stays off unless you turn it on.")
                        .size(11.0)
                        .color(palette.secondary),
                );
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new("Accounts").strong().color(palette.text));
                for account in self.source.mail().accounts() {
                    self.account_section(ui, &account, &mut sign_out);
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

    fn account_section(&mut self, ui: &mut Ui, account: &Account, sign_out: &mut Option<Account>) {
        let palette = self.palette;
        let key = account.key();
        let live = matches!(self.source, Source::Live(_));
        ui.horizontal(|ui| {
            let (spot, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(spot.center(), 5.0, palette::account(account.colour));
            ui.label(RichText::new(&account.address).strong().color(palette.text));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if live && ui.button("Sign Out").clicked() {
                    *sign_out = Some(account.clone());
                }
            });
        });
        ui.label(
            RichText::new(format!(
                "{}:{}  ·  {}:{}",
                account.imap_host, account.imap_port, account.smtp_host, account.smtp_port
            ))
            .size(11.5)
            .color(palette.secondary),
        );
        let mut changed = account.clone();
        ui.horizontal(|ui| {
            ui.label("Label");
            let label = self
                .labels
                .entry(key.clone())
                .or_insert_with(|| account.label.clone());
            if ui
                .add(
                    TextEdit::singleline(label)
                        .hint_text(account.address.as_str())
                        .desired_width(200.0),
                )
                .changed()
            {
                changed.label = label.clone();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Other addresses");
            let aliases = self
                .aliases
                .entry(key.clone())
                .or_insert_with(|| account.aliases.join(", "));
            if ui
                .add(
                    TextEdit::singleline(aliases)
                        .hint_text("hello@example.com, …")
                        .desired_width(260.0),
                )
                .changed()
            {
                changed.aliases = aliases
                    .split([',', ' ', ';'])
                    .map(str::trim)
                    .filter(|address| address.contains('@'))
                    .map(str::to_lowercase)
                    .collect();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Colour");
            for (index, colour) in palette::ACCOUNTS.iter().enumerate() {
                let (spot, response) =
                    ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::click());
                ui.painter().circle_filled(spot.center(), 7.0, *colour);
                if account.colour == Some(index as u8) {
                    ui.painter().circle_stroke(
                        spot.center(),
                        9.5,
                        egui::Stroke::new(2.0, palette.text),
                    );
                }
                if response
                    .on_hover_text(palette::ACCOUNT_NAMES[index])
                    .clicked()
                {
                    changed.colour = Some(index as u8);
                }
            }
        });
        if changed != *account {
            self.source.mail().update_account(changed);
        }
        if self.source.mail().locked(&key) {
            ui.label(
                RichText::new("Password needed: the server refused the saved password.")
                    .size(12.0)
                    .color(palette.failure),
            );
            let entry = self.reauth.entry(key.clone()).or_default();
            if let Some(reply) = &entry.1
                && let Some(result) = arrived(reply, ui.ctx())
            {
                entry.1 = None;
                match result {
                    Ok(()) => entry.0.clear(),
                    Err(error) => entry.2 = Some(error),
                }
            }
            let mut start = false;
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut entry.0)
                        .password(true)
                        .hint_text("Password or app password")
                        .desired_width(200.0),
                );
                start = ui
                    .add_enabled(
                        entry.1.is_none() && !entry.0.trim().is_empty(),
                        egui::Button::new("Sign In Again"),
                    )
                    .clicked();
                if entry.1.is_some() {
                    ui.spinner();
                }
            });
            if let Some(error) = &entry.2 {
                ui.label(RichText::new(error).size(12.0).color(palette.failure));
            }
            if start && let Source::Live(store) = &self.source {
                entry.2 = None;
                entry.1 = Some(store.sign_in_again(&key, entry.0.trim().to_string()));
            }
        }
        let blocked = self.source.mail().blocked(&key);
        if !blocked.is_empty() {
            ui.label(
                RichText::new("Blocked senders")
                    .size(12.0)
                    .color(palette.secondary),
            );
            for address in blocked {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&address).size(12.0));
                    if ui.small_button("Unblock").clicked() {
                        self.source.mail().block(&key, &address, false);
                    }
                });
            }
        }
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
        #[cfg(feature = "screenshot")]
        if let Some(title) = self.screenshot_opening.take() {
            self.open_for_screenshot(&title);
        }
        self.shortcuts(&ctx);
        self.poll_sending(&ctx);
        self.poll_fetching(&ctx);
        self.poll_thumbs(&ctx);
        self.poll_reactions(&ctx);
        if !matches!(self.source, Source::Demo(_)) && self.contacts.poll(&ctx) {
            self.known = None;
        }
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
        let dropped = files::dropped(&ctx);
        if !dropped.is_empty() {
            match &mut self.pane {
                Pane::New(composer) => composer.attach(dropped),
                Pane::Conversation => {
                    if let Some(key) = &self.selected {
                        self.replying
                            .entry(key.clone())
                            .or_default()
                            .attachments
                            .extend(dropped);
                    }
                }
            }
        }
        self.sidebar(ui, &conversations);
        match self.pane {
            Pane::Conversation if self.picked.len() > 1 => self.several(ui, &conversations),
            Pane::Conversation => self.conversation_pane(ui, &conversations),
            Pane::New(_) => self.new_pane(ui, &conversations),
        }
        self.settings_window(&ctx);
        self.confirm_window(&ctx);
        self.block_window(&ctx);
        self.empty_window(&ctx);
        self.sweep_window(&ctx);
    }
}
