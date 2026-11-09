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
(a message's HTML for Email View, its embedded images inlined), `folders.rs`
(the folder list: special-use roles, modified UTF-7 names, the order shown),
`thread.rs`
(grouping by the threading headers), `compose.rs` (reply recipients and the
outgoing message), `secrets.rs` (keyring-core) and `cache.rs`. Its worker thread
runs tokio, syncs on a timer, and publishes snapshots the window reads without
waiting. Every sync lists each account's folders and fetches INBOX and Sent;
another folder is fetched only while it is the one open, and stays cached
after. The app crate is the window: `app.rs` lays it out, `list.rs` draws the
conversation rows, `bubbles.rs` the messages and their attachment chips,
`files.rs` opening and saving attachments, `signin.rs` the account form,
`demo.rs` the sample conversations behind `--demo`, and `viewer.rs` the Email
View, a wry web view that the same binary runs as a separate process
(`zirbe --email-view [--images] <title>`, the HTML on stdin) because the egui
window owns the event loop.

## Platforms

Linux is the only platform built and shipped. Code that differs by platform sits
behind `cfg(target_os)` so the macOS and Windows lanes can be added without
restructuring: the keyring store in `store/src/secrets.rs`, the desktop settings
read over the XDG portal in `src/system.rs`, the web view's window, engine
settings and page address in `src/viewer.rs`, the empty window icon on macOS in
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
`ZIRBE_SCREENSHOT_OPEN` opens a conversation by title, `new` or `accounts`
first. The `sync` example signs in, syncs, prints the conversations and signs
out again in a scratch profile. Point `XDG_CONFIG_HOME` and `XDG_CACHE_HOME` at a scratch directory to
run against a live account without touching the real profile.

## Rules

Messages are shown as text. HTML is rendered only in Email View, which the user
opens per message, under the Content Security Policy in `src/viewer.rs`: nothing
leaves the machine unless the user chose Show Images, and then only images.
Script stays off in both cases. Plain IMAP or SMTP without TLS is never offered. Server discovery is
the offline table in `store/src/presets.rs`, never a network lookup, because
autoconfig and SRV lookups leak the address to a third party. Don't commit,
push or publish unless David asks.
