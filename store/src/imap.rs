// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_imap::Client;
use async_imap::extensions::idle::IdleResponse;
use async_imap::imap_proto::{MessageSection, SectionPath};
use async_imap::types::{Fetch, Flag, NameAttribute};
use chrono::Utc;
use futures::TryStreamExt;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::cache::{Listing, Mailbox};
use crate::folders::{self, Listed};
use crate::mime::{self, Body, Fetched};
use crate::model::{Account, FolderInfo, Role};
use crate::server::Session;

const WINDOW: u32 = 300;
const WHOLE_UP_TO: u32 = 2 * 1024 * 1024;
const BATCH: usize = 25;
const SEARCH_LIMIT: usize = 50;
const TIMEOUT: Duration = Duration::from_secs(30);
const FLAGS_EVERY: i64 = 10 * 60;

type Row = (Option<u32>, (bool, bool), u32);

pub(crate) struct Connection {
    session: async_imap::Session<TlsStream<TcpStream>>,
    condstore: Option<bool>,
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
        Ok(Connection {
            session,
            condstore: None,
        })
    }

    async fn fetch_into(
        &mut self,
        host: &str,
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
        let name = mailbox.name.clone();
        for fetch in &fetched {
            let Some(uid) = fetch.uid else { continue };
            let arrived = fetch.internal_date().map(|date| date.with_timezone(&Utc));
            let parsed = match fetch.body() {
                Some(raw) => mime::parse(&Fetched {
                    mailbox: &name,
                    uid,
                    seen: seen(fetch),
                    header: raw,
                    body: Body::Whole,
                    arrived,
                }),
                None => fetch.header().and_then(|header| {
                    mime::parse(&Fetched {
                        mailbox: &name,
                        uid,
                        seen: seen(fetch),
                        header,
                        body: Body::FirstPart {
                            mime: fetch.section(&mime_path),
                            data: fetch.section(&part_path).unwrap_or_default(),
                            attachments: fetch
                                .bodystructure()
                                .map(mime::attachment_names)
                                .unwrap_or_default(),
                            html: fetch.bodystructure().is_some_and(mime::has_html),
                            parts: fetch
                                .bodystructure()
                                .map(mime::parts_of)
                                .unwrap_or_default(),
                            encrypted: fetch.bodystructure().is_some_and(mime::is_encrypted),
                            notice: fetch.bodystructure().is_some_and(mime::is_notice),
                        },
                        arrived,
                    })
                }),
            };
            if let Some(mut message) = parsed {
                message.flagged = flagged(fetch);
                mailbox.messages.insert(uid, message);
            }
        }
        Ok(())
    }

    async fn store(&mut self, host: &str, set: &str, query: &str) -> Result<(), String> {
        let _: Vec<Fetch> = self
            .session
            .uid_store(set, query)
            .await
            .map_err(failed(host))?
            .try_collect()
            .await
            .map_err(failed(host))?;
        Ok(())
    }

    async fn expunge(&mut self, host: &str, set: &str) -> Result<(), String> {
        let capabilities = self.session.capabilities().await.map_err(failed(host))?;
        if capabilities.has_str("UIDPLUS") {
            let _: Vec<_> = self
                .session
                .uid_expunge(set)
                .await
                .map_err(failed(host))?
                .try_collect()
                .await
                .map_err(failed(host))?;
        } else {
            let _: Vec<_> = self
                .session
                .expunge()
                .await
                .map_err(failed(host))?
                .try_collect()
                .await
                .map_err(failed(host))?;
        }
        Ok(())
    }
}

impl Session for Connection {
    async fn close(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.session.logout()).await;
    }

    async fn folders(&mut self, host: &str) -> Result<Vec<FolderInfo>, String> {
        let names: Vec<_> = self
            .session
            .list(Some(""), Some("*"))
            .await
            .map_err(failed(host))?
            .try_collect()
            .await
            .map_err(failed(host))?;
        let listed = names
            .iter()
            .map(|name| {
                let attributes = name.attributes();
                Listed {
                    name: name.name().to_string(),
                    delimiter: name.delimiter().map(str::to_string),
                    selectable: !attributes.contains(&NameAttribute::NoSelect),
                    role: attributes.iter().find_map(|attribute| match attribute {
                        NameAttribute::Drafts => Some(Role::Drafts),
                        NameAttribute::Sent => Some(Role::Sent),
                        NameAttribute::Archive => Some(Role::Archive),
                        NameAttribute::Junk => Some(Role::Junk),
                        NameAttribute::Trash => Some(Role::Trash),
                        _ => None,
                    }),
                }
            })
            .collect();
        let mut folders = folders::arrange(listed);
        for folder in folders.iter_mut().filter(|folder| folder.selectable) {
            if let Ok(status) = self.session.status(&folder.name, "(UNSEEN)").await {
                folder.unread = status.unseen.unwrap_or(0);
            }
        }
        Ok(folders)
    }

    async fn sync(&mut self, host: &str, mailbox: &mut Mailbox) -> Result<Vec<u32>, String> {
        let condstore = match self.condstore {
            Some(known) => known,
            None => {
                let offered = self
                    .session
                    .capabilities()
                    .await
                    .map_err(failed(host))?
                    .has_str("CONDSTORE");
                self.condstore = Some(offered);
                offered
            }
        };
        let selected = if condstore {
            self.session.select_condstore(&mailbox.name).await
        } else {
            self.session.select(&mailbox.name).await
        }
        .map_err(failed(host))?;
        let validity = selected.uid_validity.unwrap_or(0);
        if validity != mailbox.uid_validity {
            mailbox.messages.clear();
            mailbox.found.clear();
            mailbox.listed = None;
            mailbox.uid_validity = validity;
        }
        let known = !mailbox.messages.is_empty();
        if selected.exists == 0 {
            mailbox.messages.clear();
            mailbox.listed = None;
            return Ok(Vec::new());
        }
        let listing = Listing {
            exists: selected.exists,
            uid_next: selected.uid_next.unwrap_or(0),
            modseq: selected.highest_modseq,
            at: Utc::now().timestamp(),
        };
        if known && unchanged(mailbox.listed, listing) {
            return Ok(Vec::new());
        }
        let first = selected.exists.saturating_sub(WINDOW - 1).max(1);
        let rows: Vec<Row> = self
            .session
            .fetch(format!("{first}:*"), "(UID FLAGS RFC822.SIZE)")
            .await
            .map_err(failed(host))?
            .try_collect::<Vec<Fetch>>()
            .await
            .map_err(failed(host))?
            .iter()
            .map(|fetch| {
                (
                    fetch.uid,
                    (seen(fetch), flagged(fetch)),
                    fetch.size.unwrap_or(0),
                )
            })
            .collect();
        let held = still_held(&rows);
        let present: Vec<(u32, (bool, bool), u32)> = rows
            .into_iter()
            .filter_map(|(uid, flags, size)| Some((uid?, flags, size)))
            .collect();
        if !mailbox.found.is_empty() {
            let still: BTreeSet<u32> = self
                .session
                .uid_search(format!(
                    "UID {}",
                    uid_set(&mailbox.found.iter().copied().collect::<Vec<_>>())
                ))
                .await
                .map_err(failed(host))?
                .into_iter()
                .collect();
            mailbox.found.retain(|uid| still.contains(uid));
        }
        if let Some(uids) = &held {
            let mut uids = uids.clone();
            uids.extend(&mailbox.found);
            mailbox.messages.retain(|uid, _| uids.contains(uid));
        }
        let mut whole = Vec::new();
        let mut large = Vec::new();
        for &(uid, (is_seen, is_flagged), size) in &present {
            match mailbox.messages.get_mut(&uid) {
                Some(message) => {
                    message.seen = is_seen;
                    message.flagged = is_flagged;
                }
                None if size <= WHOLE_UP_TO => whole.push(uid),
                None => large.push(uid),
            }
        }
        for batch in whole.chunks(BATCH) {
            self.fetch_into(host, mailbox, batch, "(UID FLAGS INTERNALDATE BODY.PEEK[])")
                .await?;
        }
        for batch in large.chunks(BATCH) {
            self.fetch_into(
                host,
                mailbox,
                batch,
                "(UID FLAGS INTERNALDATE BODYSTRUCTURE BODY.PEEK[HEADER] BODY.PEEK[1.MIME] BODY.PEEK[1])",
            )
            .await?;
        }
        mailbox.listed = held.is_some().then_some(listing);
        if !known {
            return Ok(Vec::new());
        }
        Ok(whole.into_iter().chain(large).collect())
    }

    async fn search_into(
        &mut self,
        host: &str,
        mailbox: &mut Mailbox,
        terms: &[String],
    ) -> Result<usize, String> {
        if terms.is_empty() {
            return Ok(0);
        }
        let selected = self
            .session
            .select(&mailbox.name)
            .await
            .map_err(failed(host))?;
        let validity = selected.uid_validity.unwrap_or(0);
        if validity != mailbox.uid_validity {
            mailbox.messages.clear();
            mailbox.found.clear();
            mailbox.listed = None;
            mailbox.uid_validity = validity;
        }
        let criteria: String = terms
            .iter()
            .map(|term| format!(" TEXT {}", quoted(term)))
            .collect();
        let mut matches: Vec<u32> = self
            .session
            .uid_search(format!("CHARSET UTF-8{criteria}"))
            .await
            .map_err(failed(host))?
            .into_iter()
            .filter(|uid| !mailbox.messages.contains_key(uid))
            .collect();
        matches.sort_unstable_by(|a, b| b.cmp(a));
        matches.truncate(SEARCH_LIMIT);
        if matches.is_empty() {
            return Ok(0);
        }
        let sizes: Vec<(u32, u32)> = self
            .session
            .uid_fetch(uid_set(&matches), "(UID RFC822.SIZE)")
            .await
            .map_err(failed(host))?
            .try_collect::<Vec<Fetch>>()
            .await
            .map_err(failed(host))?
            .iter()
            .filter_map(|fetch| Some((fetch.uid?, fetch.size.unwrap_or(0))))
            .collect();
        let (whole, large): (Vec<_>, Vec<_>) = sizes
            .into_iter()
            .partition(|(_, size)| *size <= WHOLE_UP_TO);
        let whole: Vec<u32> = whole.into_iter().map(|(uid, _)| uid).collect();
        let large: Vec<u32> = large.into_iter().map(|(uid, _)| uid).collect();
        for batch in whole.chunks(BATCH) {
            self.fetch_into(host, mailbox, batch, "(UID FLAGS INTERNALDATE BODY.PEEK[])")
                .await?;
        }
        for batch in large.chunks(BATCH) {
            self.fetch_into(
                host,
                mailbox,
                batch,
                "(UID FLAGS INTERNALDATE BODYSTRUCTURE BODY.PEEK[HEADER] BODY.PEEK[1.MIME] BODY.PEEK[1])",
            )
            .await?;
        }
        let found: Vec<u32> = whole
            .into_iter()
            .chain(large)
            .filter(|uid| mailbox.messages.contains_key(uid))
            .collect();
        mailbox.found.extend(&found);
        Ok(found.len())
    }

    async fn watch(
        mut self,
        host: &str,
        mailbox: &str,
        patience: Duration,
    ) -> Result<(Connection, u32, bool), String> {
        let selected = self.session.examine(mailbox).await.map_err(failed(host))?;
        let mut idle = self.session.idle();
        idle.init().await.map_err(failed(host))?;
        let response = {
            let (wait, _stop) = idle.wait_with_timeout(patience);
            wait.await.map_err(failed(host))?
        };
        let session = idle.done().await.map_err(failed(host))?;
        Ok((
            Connection {
                session,
                condstore: self.condstore,
            },
            selected.exists,
            matches!(response, IdleResponse::NewData(_)),
        ))
    }

    async fn whole(&mut self, host: &str, mailbox: &str, uid: u32) -> Result<Vec<u8>, String> {
        self.session.examine(mailbox).await.map_err(failed(host))?;
        let fetched: Vec<Fetch> = self
            .session
            .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")
            .await
            .map_err(failed(host))?
            .try_collect()
            .await
            .map_err(failed(host))?;
        fetched
            .iter()
            .find_map(|fetch| fetch.body().map(<[u8]>::to_vec))
            .ok_or_else(|| format!("{host} no longer has this message"))
    }

    async fn set_flag(
        &mut self,
        host: &str,
        mailbox: &str,
        uids: &[u32],
        flag: &str,
        on: bool,
    ) -> Result<(), String> {
        if uids.is_empty() {
            return Ok(());
        }
        self.session.select(mailbox).await.map_err(failed(host))?;
        let sign = if on { '+' } else { '-' };
        self.store(
            host,
            &uid_set(uids),
            &format!("{sign}FLAGS.SILENT ({flag})"),
        )
        .await
    }

    async fn move_to(
        &mut self,
        host: &str,
        mailbox: &str,
        uids: &[u32],
        destination: &str,
    ) -> Result<(), String> {
        if uids.is_empty() {
            return Ok(());
        }
        self.session.select(mailbox).await.map_err(failed(host))?;
        let capabilities = self.session.capabilities().await.map_err(failed(host))?;
        if capabilities.has_str("MOVE") {
            return self
                .session
                .uid_mv(uid_set(uids), destination)
                .await
                .map_err(failed(host));
        }
        self.session
            .uid_copy(uid_set(uids), destination)
            .await
            .map_err(failed(host))?;
        self.store(host, &uid_set(uids), "+FLAGS.SILENT (\\Deleted)")
            .await?;
        self.expunge(host, &uid_set(uids)).await
    }

    async fn delete(&mut self, host: &str, mailbox: &str, uids: &[u32]) -> Result<(), String> {
        if uids.is_empty() {
            return Ok(());
        }
        self.session.select(mailbox).await.map_err(failed(host))?;
        self.store(host, &uid_set(uids), "+FLAGS.SILENT (\\Deleted)")
            .await?;
        self.expunge(host, &uid_set(uids)).await
    }

    async fn empty(&mut self, host: &str, mailbox: &str, highest: u32) -> Result<(), String> {
        self.session.select(mailbox).await.map_err(failed(host))?;
        let set = format!("1:{highest}");
        self.store(host, &set, "+FLAGS.SILENT (\\Deleted)").await?;
        self.expunge(host, &set).await
    }

    async fn create(&mut self, host: &str, mailbox: &str) -> Result<(), String> {
        self.session.create(mailbox).await.map_err(failed(host))?;
        let _ = self.session.subscribe(mailbox).await;
        Ok(())
    }

    async fn append(&mut self, host: &str, mailbox: &str, raw: &[u8]) -> Result<(), String> {
        self.append_with(host, mailbox, raw, "(\\Seen)").await
    }

    async fn append_with(
        &mut self,
        host: &str,
        mailbox: &str,
        raw: &[u8],
        flags: &str,
    ) -> Result<(), String> {
        self.session
            .append(mailbox, Some(flags), None, raw)
            .await
            .map_err(failed(host))
    }
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn unchanged(before: Option<Listing>, now: Listing) -> bool {
    before.is_some_and(|before| {
        before.exists == now.exists
            && before.uid_next == now.uid_next
            && before.modseq == now.modseq
            && (now.modseq.is_some() || now.at - before.at < FLAGS_EVERY)
    })
}

fn still_held(rows: &[Row]) -> Option<BTreeSet<u32>> {
    rows.iter().map(|(uid, _, _)| *uid).collect()
}

fn flagged(fetch: &Fetch) -> bool {
    fetch.flags().any(|flag| flag == Flag::Flagged)
}

fn seen(fetch: &Fetch) -> bool {
    fetch.flags().any(|flag| flag == Flag::Seen)
}

#[cfg(test)]
mod tests {
    use super::{Listing, still_held, unchanged};

    fn listing(exists: u32, uid_next: u32, modseq: Option<u64>, at: i64) -> Listing {
        Listing {
            exists,
            uid_next,
            modseq,
            at,
        }
    }

    #[test]
    fn a_folder_is_unchanged_only_while_its_counts_and_modseq_hold() {
        let before = listing(10, 50, Some(700), 0);
        assert!(!unchanged(None, before));
        assert!(unchanged(Some(before), listing(10, 50, Some(700), 86_400)));
        assert!(!unchanged(Some(before), listing(10, 50, Some(701), 60)));
        assert!(!unchanged(Some(before), listing(9, 50, Some(700), 60)));
        assert!(!unchanged(Some(before), listing(10, 51, Some(700), 60)));
    }

    #[test]
    fn without_condstore_flags_are_checked_again_after_a_while() {
        let before = listing(10, 50, None, 0);
        assert!(unchanged(Some(before), listing(10, 50, None, 60)));
        assert!(!unchanged(Some(before), listing(10, 50, None, 11 * 60)));
    }

    #[test]
    fn a_listing_with_a_line_missing_its_uid_proves_nothing_gone() {
        let complete = [(Some(4), (true, false), 10), (Some(7), (false, false), 20)];
        assert_eq!(still_held(&complete), Some([4, 7].into()));
        let partial = [(Some(4), (true, false), 10), (None, (false, false), 20)];
        assert_eq!(still_held(&partial), None);
    }
}
