// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::sync::{Arc, Mutex, mpsc};

use chrono::{DateTime, Duration, Local, NaiveTime, TimeZone, Utc};
use zirbe_store::{
    Account, Conversation, Draft, Folder, MailSource, Message, Person, Reply, Status,
};

const ME: &str = "sam@example.com";

pub struct Demo {
    conversations: Mutex<Arc<Vec<Conversation>>>,
}

fn person(name: &str, address: &str) -> Person {
    Person {
        name: Some(name.to_string()),
        address: address.to_string(),
    }
}

fn me() -> Person {
    person("Sam Rivera", ME)
}

fn at(days_ago: i64, hour: u32, minute: u32) -> DateTime<Utc> {
    let date = Local::now().date_naive() - Duration::days(days_ago);
    let time = NaiveTime::from_hms_opt(hour, minute, 0).expect("a time of day");
    Local
        .from_local_datetime(&date.and_time(time))
        .earliest()
        .expect("a local time")
        .with_timezone(&Utc)
}

struct Line {
    from: Person,
    when: DateTime<Utc>,
    text: &'static str,
    attachments: &'static [&'static str],
    seen: bool,
}

fn line(from: Person, when: DateTime<Utc>, text: &'static str) -> Line {
    Line {
        from,
        when,
        text,
        attachments: &[],
        seen: true,
    }
}

fn conversation(key: &str, title: &str, members: Vec<Person>, lines: Vec<Line>) -> Conversation {
    let messages: Vec<Message> = lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let mine = line.from.is(ME);
            let to = if mine {
                members.clone()
            } else {
                members
                    .iter()
                    .filter(|person| !person.is(&line.from.address))
                    .cloned()
                    .chain(std::iter::once(me()))
                    .collect()
            };
            Message {
                folder: if mine { Folder::Sent } else { Folder::Inbox },
                uid: index as u32 + 1,
                message_id: Some(format!("<{key}.{index}@example.com>")),
                in_reply_to: None,
                references: Vec::new(),
                from: Some(line.from),
                to,
                cc: Vec::new(),
                subject: title.to_string(),
                date: line.when,
                text: line.text.to_string(),
                attachments: line
                    .attachments
                    .iter()
                    .map(|name| name.to_string())
                    .collect(),
                seen: line.seen,
                partial: false,
                html: !mine,
            }
        })
        .collect();
    let unread = messages.iter().filter(|message| !message.seen).count();
    Conversation {
        key: key.to_string(),
        account: ME.to_string(),
        title: title.to_string(),
        members,
        messages,
        unread,
    }
}

impl Demo {
    pub fn new() -> Demo {
        let ana = person("Ana Lopez", "ana@example.org");
        let ben = person("Ben Okafor", "ben@example.net");
        let mira = person("Mira Hoffmann", "mira@example.de");
        let lisa = person("Lisa Chen", "billing@cedarworks.example");
        let club = person("Priya Nair", "priya@example.org");
        let conversations = vec![
            conversation(
                "hike",
                "Saturday hike",
                vec![ana.clone(), ben.clone()],
                vec![
                    line(
                        ana.clone(),
                        at(0, 8, 12),
                        "Forecast says clear skies Saturday. Still on for the ridge trail?",
                    ),
                    line(me(), at(0, 8, 30), "Yes! I can drive. Trailhead at 7:30?"),
                    line(
                        ben.clone(),
                        at(0, 9, 2),
                        "7:30 works. I'll bring the coffee and the good map.",
                    ),
                    Line {
                        seen: false,
                        ..line(
                            ana.clone(),
                            at(0, 9, 41),
                            "Perfect. Sending the route so nobody gets lost this time.",
                        )
                    },
                    Line {
                        attachments: &["ridge-loop.gpx"],
                        seen: false,
                        ..line(
                            ana.clone(),
                            at(0, 9, 42),
                            "Eleven kilometres, mostly shade.",
                        )
                    },
                ],
            ),
            conversation(
                "invoice",
                "Invoice 2026-114",
                vec![lisa.clone()],
                vec![
                    Line {
                        attachments: &["invoice-2026-114.pdf"],
                        ..line(
                            lisa.clone(),
                            at(1, 15, 5),
                            "Hello Sam, the invoice for the October work is attached. Payment is due on the 30th.",
                        )
                    },
                    line(
                        me(),
                        at(1, 16, 20),
                        "Thanks Lisa, it's with accounts and will go out this week.",
                    ),
                ],
            ),
            conversation(
                "visit",
                "Visiting Munich",
                vec![mira.clone()],
                vec![
                    line(
                        mira.clone(),
                        at(3, 19, 45),
                        "We fixed the dates: the 12th to the 19th of December. Does the guest room still work for you?",
                    ),
                    line(
                        me(),
                        at(3, 20, 10),
                        "It does, and the Christmas market is a ten minute walk.",
                    ),
                    line(
                        mira.clone(),
                        at(2, 7, 55),
                        "Wonderful. I'll bring Lebkuchen.",
                    ),
                ],
            ),
            conversation(
                "books",
                "Book club: November",
                vec![club.clone(), ana.clone()],
                vec![
                    line(
                        club.clone(),
                        at(9, 18, 0),
                        "November's pick is The Overstory. We meet on the 24th at mine.",
                    ),
                    line(
                        ana.clone(),
                        at(9, 18, 40),
                        "Finally. I've had it on the shelf for a year.",
                    ),
                ],
            ),
        ];
        Demo {
            conversations: Mutex::new(Arc::new(conversations)),
        }
    }
}

impl MailSource for Demo {
    fn status(&self) -> Status {
        Status::Synced(Utc::now())
    }

    fn accounts(&self) -> Vec<Account> {
        vec![Account {
            address: ME.into(),
            name: "Sam Rivera".into(),
            imap_host: "imap.example.com".into(),
            imap_port: 993,
            smtp_host: "smtp.example.com".into(),
            smtp_port: 587,
        }]
    }

    fn conversations(&self) -> Arc<Vec<Conversation>> {
        Arc::clone(&self.conversations.lock().expect("demo lock"))
    }

    fn send(&self, draft: Draft) -> Reply {
        let mut guard = self.conversations.lock().expect("demo lock");
        let mut conversations: Vec<Conversation> = guard.as_ref().clone();
        let key = draft
            .in_reply_to
            .as_deref()
            .and_then(|parent| {
                conversations.iter().find(|conversation| {
                    conversation
                        .messages
                        .iter()
                        .any(|message| message.message_id.as_deref() == Some(parent))
                })
            })
            .map(|conversation| conversation.key.clone());
        let message = Message {
            folder: Folder::Sent,
            uid: 0,
            message_id: Some(format!(
                "<{}@example.com>",
                Utc::now().timestamp_nanos_opt().unwrap_or_default()
            )),
            in_reply_to: draft.in_reply_to.clone(),
            references: draft.references.clone(),
            from: Some(me()),
            to: draft.to.clone(),
            cc: draft.cc.clone(),
            subject: draft.subject.clone(),
            date: Utc::now(),
            text: draft.body.clone(),
            attachments: Vec::new(),
            seen: true,
            partial: false,
            html: false,
        };
        match key.and_then(|key| conversations.iter_mut().find(|c| c.key == key)) {
            Some(conversation) => conversation.messages.push(message),
            None => {
                let members = draft.to.iter().chain(&draft.cc).cloned().collect();
                let mut started = conversation(&draft.subject, &draft.subject, members, Vec::new());
                started.messages.push(message);
                conversations.push(started);
            }
        }
        conversations.sort_by_key(|conversation| std::cmp::Reverse(conversation.latest()));
        *guard = Arc::new(conversations);
        let (sender, receiver) = mpsc::channel();
        let _ = sender.send(Ok(()));
        receiver
    }

    fn mark_read(&self, conversation: &Conversation) {
        let mut guard = self.conversations.lock().expect("demo lock");
        let mut conversations: Vec<Conversation> = guard.as_ref().clone();
        if let Some(found) = conversations.iter_mut().find(|c| c.key == conversation.key) {
            for message in &mut found.messages {
                message.seen = true;
            }
            found.unread = 0;
        }
        *guard = Arc::new(conversations);
    }

    fn refresh(&self) {}

    fn email(&self, _account: &str, message: &Message) -> Reply<String> {
        let sender = message.from.as_ref().map_or("", |from| from.display());
        let paragraphs: String = message
            .text
            .split("\n\n")
            .map(|paragraph| format!("<p>{}</p>", paragraph.replace('\n', "<br>")))
            .collect();
        let html = format!(
            "<body style=\"margin:0;background:#f4f1ea;font:16px/1.5 sans-serif;color:#222\">\
             <div style=\"max-width:600px;margin:32px auto;background:#fff;border-radius:8px;padding:32px\">\
             <h2 style=\"margin-top:0;color:#2b674d\">{}</h2>{paragraphs}\
             <p style=\"color:#777;font-size:13px\">{sender}</p></div></body>",
            message.subject
        );
        let (reply, receiver) = mpsc::channel();
        let _ = reply.send(Ok(html));
        receiver
    }
}
