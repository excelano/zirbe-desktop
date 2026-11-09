// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use eframe::egui::{self, RichText, TextEdit, Ui};
use zirbe_store::{Account, Reply, Store};

use crate::palette::Palette;

#[derive(Default)]
pub struct SignIn {
    address: String,
    name: String,
    password: String,
    imap_host: String,
    imap_port: String,
    smtp_host: String,
    smtp_port: String,
    guessed_for: String,
    hint: Option<&'static str>,
    provider: Option<&'static str>,
    pending: Option<Reply>,
    failure: Option<String>,
}

pub enum Outcome {
    Waiting,
    SignedIn,
}

impl SignIn {
    fn fill_servers(&mut self) {
        let address = self.address.trim().to_string();
        if address == self.guessed_for {
            return;
        }
        self.guessed_for = address.clone();
        match zirbe_store::for_address(&address) {
            Some(preset) => {
                self.imap_host = preset.imap_host;
                self.imap_port = preset.imap_port.to_string();
                self.smtp_host = preset.smtp_host;
                self.smtp_port = preset.smtp_port.to_string();
                self.hint = preset.hint;
                self.provider = preset.provider;
            }
            None => {
                self.hint = None;
                self.provider = None;
            }
        }
    }

    fn account(&self) -> Option<Account> {
        Some(Account {
            address: self.address.trim().to_string(),
            name: self.name.trim().to_string(),
            imap_host: self.imap_host.trim().to_string(),
            imap_port: self.imap_port.trim().parse().ok()?,
            smtp_host: self.smtp_host.trim().to_string(),
            smtp_port: self.smtp_port.trim().parse().ok()?,
        })
    }

    pub fn show(&mut self, ui: &mut Ui, store: &Store, palette: &Palette) -> Outcome {
        if let Some(reply) = &self.pending {
            match reply.try_recv() {
                Ok(Ok(())) => {
                    *self = SignIn::default();
                    return Outcome::SignedIn;
                }
                Ok(Err(error)) => {
                    self.failure = Some(error);
                    self.pending = None;
                }
                Err(TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(200))
                }
                Err(TryRecvError::Disconnected) => self.pending = None,
            }
        }
        let width = 280.0;
        egui::Grid::new("sign in")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Email address");
                ui.add(
                    TextEdit::singleline(&mut self.address)
                        .desired_width(width)
                        .hint_text("you@example.com"),
                );
                ui.end_row();
                ui.label("Your name");
                ui.add(
                    TextEdit::singleline(&mut self.name)
                        .desired_width(width)
                        .hint_text("as others will see it"),
                );
                ui.end_row();
                ui.label(if self.hint.is_some() {
                    "App password"
                } else {
                    "Password"
                });
                ui.add(
                    TextEdit::singleline(&mut self.password)
                        .password(true)
                        .desired_width(width),
                );
                ui.end_row();
            });
        self.fill_servers();
        if let Some(hint) = self.hint {
            ui.add_space(4.0);
            ui.label(RichText::new(hint).size(12.0).color(palette.secondary));
        }
        ui.add_space(4.0);
        let label = match self.provider {
            Some(provider) => format!("Servers for {provider}"),
            None => "Servers".to_string(),
        };
        egui::CollapsingHeader::new(label).show(ui, |ui| {
            egui::Grid::new("servers")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Incoming (IMAP)");
                    server(ui, &mut self.imap_host, &mut self.imap_port);
                    ui.end_row();
                    ui.label("Outgoing (SMTP)");
                    server(ui, &mut self.smtp_host, &mut self.smtp_port);
                    ui.end_row();
                });
        });
        if let Some(failure) = &self.failure {
            ui.add_space(4.0);
            ui.label(RichText::new(failure).size(12.0).color(palette.failure));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let account = self.account().filter(|account| {
                account.address.contains('@')
                    && !account.imap_host.is_empty()
                    && !account.smtp_host.is_empty()
                    && !self.password.trim().is_empty()
            });
            let button = egui::Button::new(RichText::new("Sign In").color(palette.on_accent))
                .fill(palette.accent);
            if ui
                .add_enabled(account.is_some() && self.pending.is_none(), button)
                .clicked()
                && let Some(account) = account
            {
                self.failure = None;
                self.pending = Some(store.sign_in(account, self.password.trim().to_string()));
            }
            if self.pending.is_some() {
                ui.spinner();
                ui.label(RichText::new("Checking with the servers…").color(palette.secondary));
            }
        });
        Outcome::Waiting
    }
}

fn server(ui: &mut Ui, host: &mut String, port: &mut String) {
    ui.horizontal(|ui| {
        ui.add(TextEdit::singleline(host).desired_width(190.0));
        ui.add(TextEdit::singleline(port).desired_width(48.0));
    });
}
