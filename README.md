# Zirbe for the Linux desktop

Zirbe is an email client that reads like a texting app while keeping real email
underneath. A conversation is an email thread, not a person: messages are joined
by their `Message-ID`, `In-Reply-To` and `References` headers and shown like a
named group chat, with the subject as the title, the people on From, To and Cc
as the members, and each message as a bubble in time order. Starting a new
conversation means naming it, and that name is the subject.

This is the desktop sibling of the [Zirbe app for iPhone and iPad](https://github.com/excelano/zirbe),
with the same principles. It speaks only the email standards (IMAP, SMTP and
MIME), has no backend and no telemetry, and connects to nothing but the mail
servers you sign in to.

## What it does

The window lists your conversations beside the one that is open. Mail you sent
is read from the account's Sent folder and sits on the right, so a thread reads
the same whichever program you answered it from. A reply goes to everyone in
the conversation and carries the threading headers, so it stays in the thread
for every other mail program too. The quoted history that most mailers append
is folded away, leaving each bubble with what that message said.

It signs in to any IMAP and SMTP account with a password or an app-specific
password, and knows the servers of the large providers from the address alone.
More than one account can be signed in, and their conversations share one list.

## Install

On Debian and Ubuntu, from the Excelano apt repository:

```
curl -fsSL https://excelano.com/apt/setup.sh | sudo sh && sudo apt install zirbe
```

## Building

It is a Rust workspace and builds with a current stable toolchain:

    cargo build --release
    ./packaging/debian/build-deb.sh

The second line writes a `.deb` to `dist/` with the desktop entry, icon and
manual page. `cargo run --release -- --demo` opens sample conversations without
an account.

## First run

Sign in with your email address, your name as others should see it, and the
password. iCloud, Gmail, Yahoo, AOL and Fastmail want an app-specific password
rather than the account's own, and the sign-in form says where to make one. The
servers are filled in from the address and can be changed under Servers before
signing in. Zirbe checks both servers before it keeps anything, then saves the
password in the desktop keyring under `service=zirbe`.

## Privacy

Account settings, without the password, are kept in `~/.config/zirbe`, and the
newest messages of each inbox and Sent folder in `~/.cache/zirbe`, readable only
by you. Messages are shown as text, so nothing in an HTML message can load from
the network. The only connections are to your own IMAP and SMTP servers.

## License

MIT.
