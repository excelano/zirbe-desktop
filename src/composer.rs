// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::sync::mpsc::Receiver;

use eframe::egui::{self, Frame, Margin, RichText, TextEdit, Ui};
use zirbe_store::{Account, Draft, MailSource, Message, Original, Outgoing, Person, Reply};

use crate::files;
use crate::palette::Palette;
use crate::people;

pub enum Outcome {
    Open,
    Sent { subject: String },
    Closed,
}

#[derive(Default)]
pub struct Composer {
    pub from: String,
    to: String,
    cc: String,
    bcc: String,
    copies: bool,
    subject: String,
    body: String,
    forwarded: Option<(String, String)>,
    attachments: Vec<Outgoing>,
    picking: Option<Receiver<Vec<Outgoing>>>,
    sending: Option<Reply>,
    saving: Option<Reply>,
    failure: Option<String>,
    closing: bool,
    replaces: Option<(String, u32)>,
    heading: &'static str,
}

fn written(people: &[Person]) -> String {
    people
        .iter()
        .map(|person| match &person.name {
            Some(name) => format!("{name} <{}>", person.address),
            None => person.address.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn forwarded_block(message: &Message, text: &str) -> String {
    let sender = message
        .from
        .as_ref()
        .map_or(String::new(), |from| match &from.name {
            Some(name) => format!("{name} <{}>", from.address),
            None => from.address.clone(),
        });
    format!(
        "---------- Forwarded message ----------\nFrom: {sender}\nDate: {}\nSubject: {}\nTo: {}\n\n{text}",
        message.date.to_rfc2822(),
        message.subject,
        written(&message.to),
    )
}

impl Composer {
    pub fn new(from: String) -> Composer {
        Composer {
            from,
            heading: "New conversation",
            ..Composer::default()
        }
    }

    pub fn forward(from: String, message: &Message, original: Original) -> Composer {
        let subject = zirbe_store::subject_of(&message.subject);
        let sender = message
            .from
            .as_ref()
            .map_or("someone", |from| from.display());
        Composer {
            from,
            subject: format!("Fwd: {subject}"),
            forwarded: Some((
                format!("“{subject}” from {sender}"),
                forwarded_block(message, &original.text),
            )),
            attachments: original.attachments,
            heading: "Forward",
            ..Composer::default()
        }
    }

    pub fn draft(
        from: String,
        message: &Message,
        original: Original,
        place: (String, u32),
    ) -> Composer {
        Composer {
            from,
            to: written(&message.to),
            cc: written(&message.cc),
            copies: !message.cc.is_empty(),
            subject: message.subject.clone(),
            body: original.text,
            attachments: original.attachments,
            replaces: Some(place),
            heading: "Draft",
            ..Composer::default()
        }
    }

    pub fn attach(&mut self, files: Vec<Outgoing>) {
        self.attachments.extend(files);
    }

    fn has_content(&self) -> bool {
        [&self.to, &self.cc, &self.bcc, &self.subject, &self.body]
            .iter()
            .any(|text| !text.trim().is_empty())
            || !self.attachments.is_empty()
    }

    pub fn cancel(&mut self) -> bool {
        if self.has_content() && self.forwarded.is_none() {
            self.closing = true;
            false
        } else {
            true
        }
    }

    fn draft_value(&self) -> Result<Draft, String> {
        let mut body = self.body.trim().to_string();
        if let Some((_, block)) = &self.forwarded {
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(block);
        }
        Ok(Draft {
            account: self.from.clone(),
            to: zirbe_store::parse_people(&self.to)?,
            cc: zirbe_store::parse_people(&self.cc)?,
            bcc: zirbe_store::parse_people(&self.bcc)?,
            subject: self.subject.trim().to_string(),
            body,
            attachments: self.attachments.clone(),
            replaces: self.replaces.clone(),
            ..Draft::default()
        })
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        mail: &dyn MailSource,
        accounts: &[Account],
        known: &[Person],
        palette: &Palette,
    ) -> Outcome {
        let mut outcome = Outcome::Open;
        if let Some(reply) = &self.sending
            && let Some(result) = crate::app::arrived(reply, ui.ctx())
        {
            self.sending = None;
            match result {
                Ok(()) => {
                    outcome = Outcome::Sent {
                        subject: self.subject.trim().to_string(),
                    }
                }
                Err(error) => self.failure = Some(error),
            }
        }
        if let Some(reply) = &self.saving
            && let Some(result) = crate::app::arrived(reply, ui.ctx())
        {
            self.saving = None;
            match result {
                Ok(()) => outcome = Outcome::Closed,
                Err(error) => self.failure = Some(format!("The draft was not saved: {error}")),
            }
        }
        if let Some(picking) = &self.picking
            && let Ok(files) = picking.try_recv()
        {
            self.attachments.extend(files);
            self.picking = None;
        }
        if self.picking.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }
        egui::CentralPanel::default()
            .frame(
                Frame::new()
                    .fill(palette.canvas)
                    .inner_margin(Margin::same(24)),
            )
            .show(ui, |ui| {
                ui.label(
                    RichText::new(self.heading)
                        .size(18.0)
                        .strong()
                        .color(palette.text),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "A conversation is named by its subject, which everyone in it sees; without one it is named after its people.",
                    )
                    .size(12.0)
                    .color(palette.secondary),
                );
                ui.add_space(14.0);
                let width = ui.available_width().min(560.0);
                let field = width - 80.0;
                egui::Grid::new("new conversation")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        if accounts.len() > 1 {
                            ui.label("From");
                            ui.add_enabled_ui(self.replaces.is_none(), |ui| {
                                egui::ComboBox::from_id_salt("from")
                                    .selected_text(self.from.clone())
                                    .show_ui(ui, |ui| {
                                        for account in accounts {
                                            ui.selectable_value(
                                                &mut self.from,
                                                account.key(),
                                                account.address.clone(),
                                            );
                                        }
                                    });
                            });
                            ui.end_row();
                        }
                        ui.label("To");
                        ui.horizontal(|ui| {
                            people::field(
                                ui,
                                &mut self.to,
                                "Names and addresses, separated by commas",
                                field - 70.0,
                                "to",
                                known,
                            );
                            if !self.copies && ui.small_button("Cc/Bcc").clicked() {
                                self.copies = true;
                            }
                        });
                        ui.end_row();
                        if self.copies {
                            ui.label("Cc");
                            people::field(ui, &mut self.cc, "", field, "cc", known);
                            ui.end_row();
                            ui.label("Bcc");
                            people::field(
                                ui,
                                &mut self.bcc,
                                "Hidden from everyone else",
                                field,
                                "bcc",
                                known,
                            );
                            ui.end_row();
                        }
                        ui.label("Subject");
                        ui.add(
                            TextEdit::singleline(&mut self.subject)
                                .hint_text("Optional")
                                .desired_width(field),
                        );
                        ui.end_row();
                    });
                ui.add_space(10.0);
                let hint = if self.forwarded.is_some() {
                    "Add a note"
                } else {
                    "Message"
                };
                ui.add(
                    TextEdit::multiline(&mut self.body)
                        .hint_text(hint)
                        .desired_rows(8)
                        .desired_width(width)
                        .background_color(palette.field),
                );
                if let Some((summary, _)) = &self.forwarded {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("Forwarding {summary}"))
                            .size(12.0)
                            .color(palette.secondary),
                    );
                }
                self.attachment_row(ui, palette);
                if let Some(failure) = &self.failure {
                    ui.add_space(6.0);
                    ui.label(RichText::new(failure).size(12.0).color(palette.failure));
                }
                ui.add_space(10.0);
                if self.closing {
                    ui.label("Save this as a draft on the server?");
                    ui.horizontal(|ui| {
                        if ui.button("Save Draft").clicked() {
                            match self.draft_value() {
                                Ok(draft) => {
                                    self.failure = None;
                                    self.saving = Some(mail.save_draft(draft));
                                }
                                Err(error) => self.failure = Some(error),
                            }
                            self.closing = false;
                        }
                        if ui.button("Discard").clicked() {
                            outcome = Outcome::Closed;
                        }
                        if ui.button("Keep Editing").clicked() {
                            self.closing = false;
                        }
                    });
                    return;
                }
                ui.horizontal(|ui| {
                    let ready = !self.to.trim().is_empty()
                        && (!self.body.trim().is_empty() || self.forwarded.is_some())
                        && self.sending.is_none()
                        && self.saving.is_none();
                    let button = egui::Button::new(RichText::new("Send").color(palette.on_accent))
                        .fill(palette.accent);
                    if ui.add_enabled(ready, button).clicked() {
                        match self.draft_value() {
                            Ok(draft) => {
                                self.failure = None;
                                self.sending = Some(mail.send(draft));
                            }
                            Err(error) => self.failure = Some(error),
                        }
                    }
                    if ui.button("Cancel").clicked() && self.cancel() {
                        outcome = Outcome::Closed;
                    }
                    if self.sending.is_some() || self.saving.is_some() {
                        ui.spinner();
                    }
                });
            });
        outcome
    }

    fn attachment_row(&mut self, ui: &mut Ui, palette: &Palette) {
        ui.add_space(6.0);
        let mut removed = None;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("📎 Attach…")
                .on_hover_text("Choose files, or drop them on the window")
                .clicked()
                && self.picking.is_none()
            {
                self.picking = Some(files::pick());
            }
            for (index, file) in self.attachments.iter().enumerate() {
                if ui
                    .button(format!("{} ✕", file.name))
                    .on_hover_text("Remove")
                    .clicked()
                {
                    removed = Some(index);
                }
            }
        });
        if let Some(index) = removed {
            self.attachments.remove(index);
        }
        if files::size(&self.attachments) > files::LARGE {
            ui.label(
                RichText::new(format!(
                    "These attachments come to {} MB; many mail servers refuse messages over 25 MB.",
                    files::size(&self.attachments) / (1024 * 1024)
                ))
                .size(12.0)
                .color(palette.failure),
            );
        }
    }
}
