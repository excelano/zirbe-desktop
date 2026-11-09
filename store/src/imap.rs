// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_imap::imap_proto::{MessageSection, SectionPath};
use async_imap::types::{Fetch, Flag, NameAttribute};
use async_imap::{Client, Session};
use chrono::Utc;
use futures::TryStreamExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::cache::Mailbox;
use crate::mime::{self, Body, Fetched};
use crate::model::{Account, Folder};

const WINDOW: u32 = 300;
const WHOLE_UP_TO: u32 = 2 * 1024 * 1024;
const BATCH: usize = 25;
const TIMEOUT: Duration = Duration::from_secs(30);
const SENT_NAMES: [&str; 5] = [
    "Sent",
    "Sent Items",
    "Sent Messages",
    "Sent Mail",
    "INBOX.Sent",
];

pub(crate) struct Connection {
    session: Session<TlsStream<TcpStream>>,
}

pub(crate) fn tls() -> Result<Arc<ClientConfig>, String> {
    let mut roots = RootCertStore::empty();
    for certificate in rustls_native_certs::load_native_certs().certs {
        let _ = roots.add(certificate);
    }
    if roots.is_empty() {
        return Err("no trusted certificates were found on this computer".into());
    }
    let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(Arc::new(config))
}

async fn secure(stream: TcpStream, host: &str) -> Result<TlsStream<TcpStream>, String> {
    let name = ServerName::try_from(host.to_string())
        .map_err(|_| format!("{host} is not a server name"))?;
    TlsConnector::from(tls()?)
        .connect(name, stream)
        .await
        .map_err(|error| format!("a secure connection to {host} failed: {error}"))
}

fn failed(host: &str) -> impl Fn(async_imap::error::Error) -> String + '_ {
    move |error| format!("{host}: {error}")
}

async fn within<T>(work: impl Future<Output = Result<T, String>>, what: &str) -> Result<T, String> {
    tokio::time::timeout(TIMEOUT, work)
        .await
        .map_err(|_| format!("{what} did not answer in time"))?
}

impl Connection {
    pub async fn open(account: &Account, password: &str) -> Result<Connection, String> {
        let host = account.imap_host.trim();
        within(Self::connect(account, host, password), host).await
    }

    async fn connect(account: &Account, host: &str, password: &str) -> Result<Connection, String> {
        let tcp = TcpStream::connect((host, account.imap_port))
            .await
            .map_err(|error| format!("{host} could not be reached: {error}"))?;
        let stream = if account.imap_port == 993 {
            secure(tcp, host).await?
        } else {
            let mut plain = Client::new(tcp);
            plain
                .read_response()
                .await
                .map_err(|error| format!("{host}: {error}"))?;
            plain
                .run_command_and_check_ok("STARTTLS", None)
                .await
                .map_err(|_| {
                    format!(
                        "{host} does not offer a secure connection on port {}",
                        account.imap_port
                    )
                })?;
            secure(plain.into_inner(), host).await?
        };
        let mut client = Client::new(stream);
        if account.imap_port == 993 {
            client
                .read_response()
                .await
                .map_err(|error| format!("{host}: {error}"))?;
        }
        let session =
            client
                .login(account.key(), password)
                .await
                .map_err(|(error, _)| match error {
                    async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => {
                        format!("{host} refused the address or password")
                    }
                    other => format!("{host}: {other}"),
                })?;
        Ok(Connection { session })
    }

    pub async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.session.logout()).await;
    }

    pub async fn sent_name(&mut self, host: &str) -> Result<Option<String>, String> {
        let names: Vec<_> = self
            .session
            .list(Some(""), Some("*"))
            .await
            .map_err(failed(host))?
            .try_collect()
            .await
            .map_err(failed(host))?;
        if let Some(name) = names
            .iter()
            .find(|name| name.attributes().contains(&NameAttribute::Sent))
        {
            return Ok(Some(name.name().to_string()));
        }
        Ok(SENT_NAMES.iter().find_map(|wanted| {
            names
                .iter()
                .find(|name| {
                    let name = name.name();
                    name.eq_ignore_ascii_case(wanted)
                        || name
                            .rsplit(['/', '.'])
                            .next()
                            .is_some_and(|leaf| leaf.eq_ignore_ascii_case(wanted))
                })
                .map(|name| name.name().to_string())
        }))
    }

    pub async fn sync(
        &mut self,
        host: &str,
        folder: Folder,
        mailbox: &mut Mailbox,
    ) -> Result<(), String> {
        let selected = self
            .session
            .select(&mailbox.name)
            .await
            .map_err(failed(host))?;
        let validity = selected.uid_validity.unwrap_or(0);
        if validity != mailbox.uid_validity {
            mailbox.messages.clear();
            mailbox.uid_validity = validity;
        }
        if selected.exists == 0 {
            mailbox.messages.clear();
            return Ok(());
        }
        let first = selected.exists.saturating_sub(WINDOW - 1).max(1);
        let present: Vec<(u32, bool, u32)> = self
            .session
            .fetch(format!("{first}:*"), "(UID FLAGS RFC822.SIZE)")
            .await
            .map_err(failed(host))?
            .try_collect::<Vec<Fetch>>()
            .await
            .map_err(failed(host))?
            .iter()
            .filter_map(|fetch| Some((fetch.uid?, seen(fetch), fetch.size.unwrap_or(0))))
            .collect();
        let uids: BTreeSet<u32> = present.iter().map(|(uid, _, _)| *uid).collect();
        mailbox.messages.retain(|uid, _| uids.contains(uid));
        let mut whole = Vec::new();
        let mut large = Vec::new();
        for &(uid, is_seen, size) in &present {
            match mailbox.messages.get_mut(&uid) {
                Some(message) => message.seen = is_seen,
                None if size <= WHOLE_UP_TO => whole.push(uid),
                None => large.push(uid),
            }
        }
        for batch in whole.chunks(BATCH) {
            self.fetch_into(
                host,
                folder,
                mailbox,
                batch,
                "(UID FLAGS INTERNALDATE BODY.PEEK[])",
            )
            .await?;
        }
        for batch in large.chunks(BATCH) {
            self.fetch_into(
                host,
                folder,
                mailbox,
                batch,
                "(UID FLAGS INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[1.MIME] BODY.PEEK[1])",
            )
            .await?;
        }
        Ok(())
    }

    async fn fetch_into(
        &mut self,
        host: &str,
        folder: Folder,
        mailbox: &mut Mailbox,
        uids: &[u32],
        query: &str,
    ) -> Result<(), String> {
        let set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let fetched: Vec<Fetch> = self
            .session
            .uid_fetch(set, query)
            .await
            .map_err(failed(host))?
            .try_collect()
            .await
            .map_err(failed(host))?;
        let mime_path = SectionPath::Part(vec![1], Some(MessageSection::Mime));
        let part_path = SectionPath::Part(vec![1], None);
        for fetch in &fetched {
            let Some(uid) = fetch.uid else { continue };
            let arrived = fetch.internal_date().map(|date| date.with_timezone(&Utc));
            let parsed = match fetch.body() {
                Some(raw) => mime::parse(&Fetched {
                    folder,
                    uid,
                    seen: seen(fetch),
                    header: raw,
                    body: Body::Whole,
                    arrived,
                }),
                None => fetch.header().and_then(|header| {
                    mime::parse(&Fetched {
                        folder,
                        uid,
                        seen: seen(fetch),
                        header,
                        body: Body::FirstPart {
                            mime: fetch.section(&mime_path),
                            data: fetch.section(&part_path).unwrap_or_default(),
                        },
                        arrived,
                    })
                }),
            };
            if let Some(message) = parsed {
                mailbox.messages.insert(uid, message);
            }
        }
        Ok(())
    }

    pub async fn mark_seen(
        &mut self,
        host: &str,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<(), String> {
        if uids.is_empty() {
            return Ok(());
        }
        self.session.select(mailbox).await.map_err(failed(host))?;
        let set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let _: Vec<Fetch> = self
            .session
            .uid_store(set, "+FLAGS.SILENT (\\Seen)")
            .await
            .map_err(failed(host))?
            .try_collect()
            .await
            .map_err(failed(host))?;
        Ok(())
    }

    pub async fn append(&mut self, host: &str, mailbox: &str, raw: &[u8]) -> Result<(), String> {
        self.session
            .append(mailbox, Some("(\\Seen)"), None, raw)
            .await
            .map_err(failed(host))
    }
}

fn seen(fetch: &Fetch) -> bool {
    fetch.flags().any(|flag| flag == Flag::Seen)
}
