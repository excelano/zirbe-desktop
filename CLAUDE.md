# CLAUDE.md

Zirbe for the Linux desktop: an IMAP and SMTP account read as conversations, an
egui application. It is the desktop sibling of excelano/zirbe, the iPhone and
iPad app, and keeps that app's model and principles: a conversation is an email
thread joined by Message-ID, In-Reply-To and References; the subject is its
name; only the email standards are implemented; the only network destinations
are the user's own mail servers.

## Layout

The store crate, `store/` (`zirbe-store`), is the mail engine behind
`MailSource`: `imap.rs` (async-imap over tokio-rustls), `smtp.rs` (lettre),
`mime.rs` and `content.rs` (mail-parser, html2text, quote folding), `email.rs`
(a message's HTML for Web View, its embedded images inlined), `folders.rs`
(the folder list: special-use roles, modified UTF-7 names, the order shown),
`actions.rs` (which copies of a conversation's messages a change touches),
`search.rs` (word-start matching that ignores case and accents), `invite.rs`
(the text/calendar reader behind invitation cards),
`thread.rs` (grouping by the threading headers, keeping every copy of a message
filed in several folders), `compose.rs` (reply recipients and the
outgoing message), `secrets.rs` (keyring-core) and `cache.rs`. Its worker thread
runs tokio, syncs on a timer, and publishes snapshots the window reads without
waiting. Every sync lists each account's folders and fetches INBOX and Sent;
another folder is fetched only while it is the one open, and stays cached
after. Besides the two-minute timer, a task per account holds INBOX in IMAP
IDLE in 25-second rounds and asks for that account's sync when the server
reports a change or the message count moves; new unread mail from others found
by a sync is queued for the window as arrivals. A sent message's raw text stays
in the cache until its copy is seen in the Sent folder, and every sync retries
saving it there. The app crate is the window: `app.rs` lays it out, `list.rs` draws the
conversation rows, `filter.rs` the list's filter, `bubbles.rs` the messages and their attachment chips,
`files.rs` opening, saving, picking and dropping files, `composer.rs` the
new-conversation, forward and draft pane, `people.rs` recipient suggestions
drawn from the synced mail and `contacts.rs` the desktop address book
(Evolution Data Server over the session bus: names and addresses for
suggestions, photos for avatars from the card or a local file and never a URL,
checked each minute and reread when a book's revision changes), `notify.rs` new-mail notifications
and the launcher's unread count over the session bus, `signin.rs` the account form,
`demo.rs` the sample conversations behind `--demo`, and `page.rs` the Email
View: Blitz (`blitz-html`, pinned to an exact beta) lays out and paints a
message's HTML on its own thread into tiles the reading pane draws, and its
`NetProvider` is the only way the page reaches anything outside the message.

## Platforms

Linux is the only platform built and shipped. Code that differs by platform sits
behind `cfg(target_os)` so the macOS and Windows lanes can be added without
restructuring: the keyring store in `store/src/secrets.rs`, the desktop settings
read over the XDG portal in `src/system.rs`, the empty window icon on macOS in
`src/main.rs`, and the Windows manifest that `build.rs` embeds. Paths come from
`dirs`. `ring` compiles C, so a cross-target `cargo check` from Linux needs that
platform's SDK headers; check those arms on the Mac and on winwork.
`~/notes/desktop_app_from_the_start.md` lists what each store lane still needs.

## Commands

    cargo test --workspace
    cargo run -- --demo
    ZIRBE_PASSWORD=... cargo run --example sync -p zirbe-store -- <address>
    ./packaging/debian/build-deb.sh
    ./packaging/linux/check-libraries.sh

The `screenshot` feature lets a build photograph its own window:
`ZIRBE_SCREENSHOT=<png>` saves one after `ZIRBE_SCREENSHOT_DELAY` seconds and
closes, `ZIRBE_SCREENSHOT_SIZE=<w>x<h>` sizes the window, and
`ZIRBE_SCREENSHOT_OPEN` opens a conversation by title, `new` or `settings`
first. To drive it with keys and clicks, run it on `Xvfb` with `DISPLAY`
pointed there and `WAYLAND_DISPLAY` unset, or winit opens the window on the
session's Wayland display and `xdotool` reaches nothing; with no window
manager on `Xvfb`, clicks go to the window under the pointer, but keys reach
nothing until `xdotool windowfocus` gives the window the keyboard. The `sync` example signs in, syncs, prints the conversations and signs
out again in a scratch profile. Point `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` at a scratch directory to
run against a live account without touching the real profile.

## Rules

Messages are shown as text. HTML is rendered only in Web View, which the user
opens per message, and Blitz runs no script. The page's `NetProvider` in
`src/page.rs` answers images embedded in the message and nothing else, unless
the user chose Show Images; then it fetches the message's `<img>` sources and
nothing more, and keeps a response only if it is an image. Plain IMAP or SMTP without TLS is never offered. Server discovery is
the offline table in `store/src/presets.rs`, never a network lookup, because
autoconfig and SRV lookups leak the address to a third party. Don't commit,
push or publish unless David asks.
