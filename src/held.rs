// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::time::{Duration, Instant};

use eframe::egui;
use zirbe_store::{Draft, MailSource, Reply};

pub struct Held {
    pub draft: Draft,
    since: Instant,
    delay: Duration,
    reply: Option<Reply>,
}

impl Held {
    pub fn new(draft: Draft, delay: Duration, mail: &dyn MailSource) -> Held {
        let reply = delay.is_zero().then(|| mail.send(draft.clone()));
        Held {
            draft,
            since: Instant::now(),
            delay,
            reply,
        }
    }

    pub fn waiting(&self) -> Option<Duration> {
        self.reply
            .is_none()
            .then(|| self.delay.saturating_sub(self.since.elapsed()))
    }

    pub fn poll(
        &mut self,
        mail: &dyn MailSource,
        ctx: &egui::Context,
    ) -> Option<Result<(), String>> {
        if let Some(left) = self.waiting() {
            if left.is_zero() {
                self.reply = Some(mail.send(self.draft.clone()));
            } else {
                ctx.request_repaint_after(left.min(Duration::from_millis(250)));
                return None;
            }
        }
        crate::app::arrived(self.reply.as_ref()?, ctx)
    }
}
