// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions};
use image::imageops::FilterType;
use zirbe_store::Person;

const STALE: Duration = Duration::from_secs(60);
const FACE: u32 = 96;

#[derive(Default)]
pub struct Contacts {
    people: Vec<Person>,
    faces: Faces,
    stamp: String,
    loaded: Option<Instant>,
    loading: Option<Receiver<Option<Book>>>,
}

#[derive(Default)]
pub struct Faces(HashMap<String, TextureHandle>);

impl Faces {
    pub fn of(&self, person: Option<&Person>) -> Option<&TextureHandle> {
        person.and_then(|person| self.0.get(&person.address.to_lowercase()))
    }
}

struct Book {
    stamp: String,
    people: Vec<Person>,
    faces: Vec<(Vec<String>, ColorImage)>,
}

#[derive(Debug, Default, PartialEq)]
struct Card {
    name: Option<String>,
    addresses: Vec<String>,
    photo: Option<Vec<u8>>,
}

impl Contacts {
    pub fn people(&self) -> &[Person] {
        &self.people
    }

    pub fn faces(&self) -> &Faces {
        &self.faces
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> bool {
        let mut changed = false;
        if let Some(loading) = &self.loading {
            match loading.try_recv() {
                Ok(book) => {
                    if let Some(book) = book {
                        changed = book.people != self.people;
                        self.take(ctx, book);
                    }
                    self.loaded = Some(Instant::now());
                    self.loading = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.loaded = Some(Instant::now());
                    self.loading = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.loading.is_none() && self.loaded.is_none_or(|at| at.elapsed() > STALE) {
            let (sender, receiver) = mpsc::channel();
            let stamp = self.stamp.clone();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new()
                .name("contacts".into())
                .spawn(move || {
                    let _ = sender.send(load(&stamp));
                    ctx.request_repaint();
                });
            if spawned.is_ok() {
                self.loading = Some(receiver);
            } else {
                self.loaded = Some(Instant::now());
            }
        }
        changed
    }

    fn take(&mut self, ctx: &egui::Context, book: Book) {
        let mut faces = HashMap::new();
        for (index, (addresses, image)) in book.faces.into_iter().enumerate() {
            let texture = ctx.load_texture(format!("face {index}"), image, TextureOptions::LINEAR);
            for address in addresses {
                faces.insert(address, texture.clone());
            }
        }
        self.faces = Faces(faces);
        self.people = book.people;
        self.stamp = book.stamp;
    }
}

impl Book {
    fn new(stamp: String, cards: Vec<Card>) -> Book {
        let mut people = Vec::new();
        let mut faces = Vec::new();
        for card in cards {
            if let Some(image) = card.photo.as_deref().and_then(face) {
                let addresses = card.addresses.iter().map(|a| a.to_lowercase()).collect();
                faces.push((addresses, image));
            }
            people.extend(card.addresses.into_iter().map(|address| Person {
                name: card.name.clone(),
                address,
            }));
        }
        Book {
            stamp,
            people,
            faces,
        }
    }
}

fn face(bytes: &[u8]) -> Option<ColorImage> {
    let image = image::load_from_memory(bytes)
        .ok()?
        .resize_to_fill(FACE, FACE, FilterType::Triangle)
        .to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    Some(ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
}

#[cfg(target_os = "linux")]
fn load(stamp: &str) -> Option<Book> {
    eds::read(stamp).ok().flatten()
}

#[cfg(not(target_os = "linux"))]
fn load(_stamp: &str) -> Option<Book> {
    None
}

#[cfg(target_os = "linux")]
mod eds {
    use std::collections::HashMap;

    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    use super::{Book, Card};

    const SOURCES: &str = "org.gnome.evolution.dataserver.Sources5";
    const SOURCE_MANAGER: &str = "/org/gnome/evolution/dataserver/SourceManager";
    const SOURCE: &str = "org.gnome.evolution.dataserver.Source";
    const BOOKS: &str = "org.gnome.evolution.dataserver.AddressBook10";
    const BOOK_FACTORY: &str = "/org/gnome/evolution/dataserver/AddressBookFactory";
    const BOOK: &str = "org.gnome.evolution.dataserver.AddressBook";
    const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

    type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    struct Opened {
        uid: String,
        revision: String,
        bus: String,
        path: String,
    }

    pub(super) fn read(stamp: &str) -> zbus::Result<Option<Book>> {
        let connection = Connection::session()?;
        let mut books: Vec<Opened> = enabled_books(&connection)?
            .iter()
            .filter_map(|uid| open(&connection, uid).ok())
            .collect();
        books.sort();
        let fresh: String = books
            .iter()
            .map(|book| format!("{}={};", book.uid, book.revision))
            .collect();
        let read = (fresh != stamp).then(|| {
            let cards = books
                .iter()
                .flat_map(|book| cards(&connection, book).unwrap_or_default())
                .collect();
            Book::new(fresh, cards)
        });
        for book in &books {
            let _ = call(&connection, book, BOOK, "Close", &());
        }
        Ok(read)
    }

    fn enabled_books(connection: &Connection) -> zbus::Result<Vec<String>> {
        let reply = connection.call_method(
            Some(SOURCES),
            SOURCE_MANAGER,
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )?;
        let objects: Objects = reply.body().deserialize()?;
        let text = |value: Option<OwnedValue>| value.and_then(|v| String::try_from(v).ok());
        Ok(objects
            .into_values()
            .filter_map(|mut interfaces| {
                let mut source = interfaces.remove(SOURCE)?;
                let uid = text(source.remove("UID"))?;
                let data = text(source.remove("Data"))?;
                is_enabled_book(&data).then_some(uid)
            })
            .collect())
    }

    fn is_enabled_book(data: &str) -> bool {
        let lines = || data.lines().map(str::trim);
        lines().any(|line| line == "[Address Book]") && !lines().any(|line| line == "Enabled=false")
    }

    fn open(connection: &Connection, uid: &str) -> zbus::Result<Opened> {
        let reply = connection.call_method(
            Some(BOOKS),
            BOOK_FACTORY,
            Some("org.gnome.evolution.dataserver.AddressBookFactory"),
            "OpenAddressBook",
            &uid,
        )?;
        let (path, bus): (String, String) = reply.body().deserialize()?;
        let mut book = Opened {
            uid: uid.to_string(),
            revision: String::new(),
            bus,
            path,
        };
        call(connection, &book, BOOK, "Open", &())?;
        let revision: OwnedValue = call(connection, &book, PROPERTIES, "Get", &(BOOK, "Revision"))?
            .body()
            .deserialize()?;
        book.revision = String::try_from(revision)?;
        Ok(book)
    }

    fn cards(connection: &Connection, book: &Opened) -> zbus::Result<Vec<Card>> {
        let query = "(exists \"email\")";
        let cards: Vec<String> = call(connection, book, BOOK, "GetContactList", &query)?
            .body()
            .deserialize()?;
        Ok(cards.iter().map(|card| super::card(card)).collect())
    }

    fn call<B>(
        connection: &Connection,
        book: &Opened,
        interface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        connection.call_method(
            Some(book.bus.as_str()),
            book.path.as_str(),
            Some(interface),
            method,
            body,
        )
    }
}

fn card(text: &str) -> Card {
    let unfolded = text
        .replace("\r\n", "\n")
        .replace("\n ", "")
        .replace("\n\t", "");
    let mut card = Card::default();
    for line in unfolded.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let mut parts = key.split(';');
        let property = parts.next().unwrap_or_default();
        let property = property.rsplit('.').next().unwrap_or_default();
        if property.eq_ignore_ascii_case("PHOTO") {
            let inline = parts.any(|part| {
                let part = part.to_ascii_uppercase();
                part == "ENCODING=B" || part == "ENCODING=BASE64"
            });
            card.photo = photo(value, inline).or(card.photo);
            continue;
        }
        let value = unescape(value);
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if property.eq_ignore_ascii_case("FN") {
            card.name = Some(value.to_string());
        } else if property.eq_ignore_ascii_case("EMAIL") && value.contains('@') {
            card.addresses.push(value.to_string());
        }
    }
    card
}

fn photo(value: &str, inline: bool) -> Option<Vec<u8>> {
    let value = value.trim();
    if inline {
        return STANDARD.decode(value).ok();
    }
    if let Some((meta, data)) = value.split_once(',')
        && meta.starts_with("data:")
        && meta.ends_with(";base64")
    {
        return STANDARD.decode(data).ok();
    }
    let path = url::Url::parse(value)
        .ok()
        .filter(|url| url.scheme() == "file")?
        .to_file_path()
        .ok()?;
    std::fs::read(path).ok()
}

fn unescape(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            text.push(character);
            continue;
        }
        match characters.next() {
            Some('n' | 'N') => text.push(' '),
            Some(escaped) => text.push(escaped),
            None => {}
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_each_address_under_the_full_name() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Pat Lee\\, PhD\r\nN:Lee;Pat;;;\r\n\
            EMAIL;TYPE=WORK:pat@work.example\r\nitem1.EMAIL;TYPE=HOME:pat@ho\r\n me.example\r\n\
            EMAIL:\r\nEND:VCARD\r\n";
        assert_eq!(
            card(text),
            Card {
                name: Some("Pat Lee, PhD".into()),
                addresses: vec!["pat@work.example".into(), "pat@home.example".into()],
                photo: None,
            }
        );
    }

    #[test]
    fn leaves_a_card_without_a_name_unnamed() {
        let read = card("BEGIN:VCARD\nEMAIL:sam@example.com\nEND:VCARD\n");
        assert_eq!(read.name, None);
        assert_eq!(read.addresses, vec!["sam@example.com".to_string()]);
    }

    #[test]
    fn reads_an_inline_photo_folded_over_lines() {
        let text = "BEGIN:VCARD\r\nPHOTO;ENCODING=b;TYPE=PNG:aGVs\r\n bG8=\r\nEND:VCARD\r\n";
        assert_eq!(card(text).photo, Some(b"hello".to_vec()));
    }

    #[test]
    fn reads_a_data_uri_photo() {
        let text = "BEGIN:VCARD\nPHOTO:data:image/png;base64,aGVsbG8=\nEND:VCARD\n";
        assert_eq!(card(text).photo, Some(b"hello".to_vec()));
    }

    #[test]
    fn never_fetches_a_photo_from_the_network() {
        let text = "BEGIN:VCARD\nPHOTO;VALUE=uri:https://example.com/pat.jpg\nEND:VCARD\n";
        assert_eq!(card(text).photo, None);
    }
}
