// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::HashMap;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use eframe::egui::{
    self, Align, Frame, Key, Layout, Margin, Modifiers, RichText, ScrollArea, TextEdit, Ui,
};
use zirbe_store::{Account, Conversation, Draft, MailSource, Reply, Status, Store};

use crate::bubbles::{self, Opening};
use crate::dates;
use crate::demo::Demo;
use crate::list;
use crate::palette::Palette;
use crate::signin::{Outcome, SignIn};
use crate::system::Clock;
use crate::viewer;

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

struct Viewing {
    conversation: String,
    title: String,
    images: bool,
    reply: Reply<String>,
}

pub struct App {
    source: Source,
    palette: Palette,
    clock: Clock,
    selected: Option<String>,
    drafts: HashMap<String, String>,
    sending: Option<Sending>,
    viewing: Option<Viewing>,
    failure: Option<(String, String)>,
    pane: Pane,
    accounts_open: bool,
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
            viewing: None,
            failure: None,
            pane: Pane::Conversation,
            accounts_open: false,
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
            "accounts" => self.accounts_open = true,
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
        if matches!(self.pane, Pane::New(_))
            && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.pane = Pane::Conversation;
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

    fn poll_viewing(&mut self, ctx: &egui::Context) {
        let Some(viewing) = &self.viewing else { return };
        let result = match viewing.reply.try_recv() {
            Ok(Ok(html)) => viewer::open(&viewing.title, html, viewing.images),
            Ok(Err(error)) => Err(error),
            Err(TryRecvError::Empty) => {
                ctx.request_repaint_after(Duration::from_millis(150));
                return;
            }
            Err(TryRecvError::Disconnected) => Ok(()),
        };
        if let Err(error) = result {
            self.failure = Some((
                viewing.conversation.clone(),
                format!("The email did not open. {error}"),
            ));
        }
        self.viewing = None;
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
                    ui.add_space(14.0);
                    ui.label(
                        RichText::new("Conversations")
                            .size(17.0)
                            .strong()
                            .color(palette.text),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(10.0);
                        if ui.button("⚙").on_hover_text("Accounts").clicked() {
                            self.accounts_open = true;
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
                egui::Panel::bottom("status")
                    .frame(
                        Frame::new()
                            .fill(palette.sidebar)
                            .inner_margin(Margin::symmetric(14, 6)),
                    )
                    .show(ui, |ui| self.status(ui));
                ScrollArea::vertical().auto_shrink(false).show_rows(
                    ui,
                    list::ROW_HEIGHT,
                    conversations.len(),
                    |ui, range| {
                        let today = dates::today();
                        for conversation in &conversations[range] {
                            let selected = matches!(self.pane, Pane::Conversation)
                                && self.selected.as_deref() == Some(conversation.key.as_str());
                            if list::row(ui, conversation, selected, today, self.clock, &palette)
                                .clicked()
                            {
                                self.selected = Some(conversation.key.clone());
                                self.pane = Pane::Conversation;
                            }
                        }
                    },
                );
            });
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
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(list::title(&conversation))
                            .size(16.0)
                            .strong()
                            .color(palette.text),
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
        egui::Panel::bottom("composer")
            .frame(
                Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::symmetric(16, 10)),
            )
            .show(ui, |ui| self.composer(ui, &conversation, account.as_ref()));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(palette.canvas))
            .show(ui, |ui| {
                let me = account
                    .as_ref()
                    .map_or(conversation.account.as_str(), |account| {
                        account.address.as_str()
                    });
                let opening = ScrollArea::vertical()
                    .id_salt(&conversation.key)
                    .stick_to_bottom(true)
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        ui.add_space(8.0);
                        bubbles::show(ui, &conversation, me, dates::today(), self.clock, &palette)
                    })
                    .inner;
                if let Some(Opening { message, images }) = opening {
                    self.failure = None;
                    self.viewing = Some(Viewing {
                        conversation: conversation.key.clone(),
                        title: list::title(&conversation).to_string(),
                        images,
                        reply: self.source.mail().email(&conversation.account, &message),
                    });
                }
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
        if self
            .viewing
            .as_ref()
            .is_some_and(|viewing| viewing.conversation == conversation.key)
        {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Opening the email…")
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

    fn accounts_window(&mut self, ctx: &egui::Context) {
        if !self.accounts_open {
            return;
        }
        let palette = self.palette;
        let mut open = true;
        let mut sign_out = None;
        egui::Window::new("Accounts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
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
            store.sign_out(&account);
        }
        self.accounts_open = open;
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
        self.poll_viewing(&ctx);
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
        self.sidebar(ui, &conversations);
        match self.pane {
            Pane::Conversation => self.conversation_pane(ui, &conversations),
            Pane::New(_) => self.new_pane(ui),
        }
        self.accounts_window(&ctx);
    }
}
