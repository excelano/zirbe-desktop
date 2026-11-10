// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::sync::mpsc::{self, Receiver};

use eframe::egui;
use zirbe_store::Arrival;

const SUMMARY_PAST: usize = 3;

pub struct Notifier {
    clicks: Receiver<Option<String>>,
    #[cfg(target_os = "linux")]
    desktop: Option<linux::Desktop>,
    badge: Option<u32>,
}

impl Notifier {
    pub fn new(ctx: &egui::Context) -> Notifier {
        let (sender, clicks) = mpsc::channel();
        #[cfg(target_os = "linux")]
        let desktop = linux::Desktop::connect(sender, ctx.clone());
        #[cfg(not(target_os = "linux"))]
        let _ = (sender, ctx);
        Notifier {
            clicks,
            #[cfg(target_os = "linux")]
            desktop,
            badge: None,
        }
    }

    pub fn post(&mut self, arrivals: &[Arrival]) {
        let notes: Vec<(String, String, Option<String>)> = if arrivals.len() > SUMMARY_PAST {
            let senders: Vec<&str> = arrivals.iter().map(|a| a.sender.as_str()).collect();
            vec![(
                format!("{} new messages", arrivals.len()),
                format!("From {}", senders.join(", ")),
                None,
            )]
        } else {
            arrivals
                .iter()
                .map(|arrival| {
                    let text = arrival
                        .text
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    (
                        arrival.sender.clone(),
                        format!("{}\n{text}", arrival.subject),
                        arrival.message_id.clone(),
                    )
                })
                .collect()
        };
        #[cfg(target_os = "linux")]
        if let Some(desktop) = &mut self.desktop {
            for (summary, body, message_id) in notes {
                desktop.notify(&summary, &body, message_id);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = notes;
    }

    pub fn clicked(&self) -> Option<Option<String>> {
        self.clicks.try_recv().ok()
    }

    pub fn badge(&mut self, count: u32) {
        if self.badge == Some(count) {
            return;
        }
        self.badge = Some(count);
        #[cfg(target_os = "linux")]
        if let Some(desktop) = &self.desktop {
            desktop.badge(count);
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, mpsc::Sender};

    use eframe::egui;
    use zbus::blocking::{Connection, MessageIterator};
    use zbus::zvariant::Value;
    use zbus::{MatchRule, message};

    const SERVICE: &str = "org.freedesktop.Notifications";
    const PATH: &str = "/org/freedesktop/Notifications";
    const LAUNCHER: &str = "/com/canonical/unity/launcherentry/zirbe";

    pub struct Desktop {
        connection: Connection,
        posted: Arc<Mutex<HashMap<u32, Option<String>>>>,
    }

    impl Desktop {
        pub fn connect(clicks: Sender<Option<String>>, ctx: egui::Context) -> Option<Desktop> {
            let connection = Connection::session().ok()?;
            let posted: Arc<Mutex<HashMap<u32, Option<String>>>> = Arc::default();
            let rule = MatchRule::builder()
                .msg_type(message::Type::Signal)
                .interface(SERVICE)
                .ok()?
                .member("ActionInvoked")
                .ok()?
                .build();
            let listener = connection.clone();
            let known = Arc::clone(&posted);
            std::thread::Builder::new()
                .name("notification clicks".into())
                .spawn(move || {
                    let Ok(signals) = MessageIterator::for_match_rule(rule, &listener, None) else {
                        return;
                    };
                    for signal in signals.flatten() {
                        let Ok((id, action)) = signal.body().deserialize::<(u32, String)>() else {
                            continue;
                        };
                        let target = known
                            .lock()
                            .ok()
                            .and_then(|posted| posted.get(&id).cloned());
                        if action == "default"
                            && let Some(target) = target
                        {
                            if clicks.send(target).is_err() {
                                return;
                            }
                            ctx.request_repaint();
                        }
                    }
                })
                .ok()?;
            Some(Desktop { connection, posted })
        }

        pub fn notify(&mut self, summary: &str, body: &str, message_id: Option<String>) {
            let hints: HashMap<&str, Value<'_>> = HashMap::from([
                ("desktop-entry", Value::from("zirbe")),
                ("category", Value::from("email.arrived")),
            ]);
            let sent = self.connection.call_method(
                Some(SERVICE),
                PATH,
                Some(SERVICE),
                "Notify",
                &(
                    "Zirbe",
                    0u32,
                    "zirbe",
                    summary,
                    body,
                    vec!["default", "Open"],
                    hints,
                    -1i32,
                ),
            );
            if let Ok(reply) = sent
                && let Ok(id) = reply.body().deserialize::<u32>()
                && let Ok(mut posted) = self.posted.lock()
            {
                posted.insert(id, message_id);
            }
        }

        pub fn badge(&self, count: u32) {
            let properties: HashMap<&str, Value<'_>> = HashMap::from([
                ("count", Value::from(i64::from(count))),
                ("count-visible", Value::from(count > 0)),
            ]);
            let _ = self.connection.emit_signal(
                None::<&str>,
                LAUNCHER,
                "com.canonical.Unity.LauncherEntry",
                "Update",
                &("application://zirbe.desktop", properties),
            );
        }
    }
}
