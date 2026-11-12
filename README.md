# Zirbe for the Linux desktop

Zirbe is an email client that reads like a texting app while keeping real email
underneath. A conversation is an email thread, not a person: messages are joined
by their `Message-ID`, `In-Reply-To` and `References` headers and shown like a
named group chat, with the subject as the title, the people on From, To and Cc
as the members, and each message as a bubble in time order. A private reply
out of a thread, such as an answer to a message sent to many people by Bcc,
starts a conversation of its own with the message it answers shown at the top;
an account's other addresses, set in Settings, count as yours. Starting a new
conversation means naming it, and that name is the subject.

This is the desktop sibling of the [Zirbe app for iPhone and iPad](https://github.com/excelano/zirbe),
with the same principles. It speaks only the email standards (IMAP, SMTP and
MIME), has no backend and no telemetry, and connects to nothing but the mail
servers you sign in to.

## What it does

The window lists your conversations beside the one that is open. The list
starts on All Inboxes, every account's inbox together, and the menu at its top
switches to one account's folder instead; a conversation shows in each folder
holding one of its messages, always as the whole thread. A conversation can be
archived, trashed, moved, sent to Junk, flagged, pinned to the top or marked
read or unread from the buttons above it, from its right-click menu, or with
E for Archive, Delete for Trash and Shift+U for read or unread. The Up and Down
arrows step through the list; Shift with an arrow, Shift-click and Ctrl-click
pick several at once, and Ctrl+A or Select All in the ⋯ menu above the list
picks every conversation shown, which the same menu can also mark read. Filter
at the foot of the list narrows it to conversations that are unread or read,
plain text or HTML, from people or from mailing lists, flagged, or carrying
attachments. Archive makes an Archive folder if the account has none, Delete
inside Trash removes for good after asking, and Empty Trash or Empty Junk at
the top of those folders deletes everything in them after asking, keeping any
mail that arrived since the last check. New mail in an inbox shows
within seconds, and while Zirbe is open but not in front it posts a
notification for each new message, which opens the conversation when clicked;
Settings can turn the notifications off. The search field above the list
(Ctrl+F) finds conversations by words or the start of words in their subject,
people and messages, ignoring case and accents, and can then ask the server for
older matches in the folder being shown, which Zirbe keeps once found; Find in
Conversation (Ctrl+Shift+F) steps through the messages of one conversation that
match. Right-clicking a message replies to that message alone, forwards it with
its attachments, copies its text, or shows the quoted history Zirbe folds away.
Above the reply box are the people a reply reaches, any of whom can be left out
of it, and files can be attached with the paperclip or by dropping them on the
window. A new conversation has Cc and Bcc, takes its name from its people when
it has no subject, suggests addresses from the mail already synced and from the
desktop's address book, and can be
saved to the server's Drafts folder instead of sent; opening a draft from the
Drafts folder brings it back to finish. A reply that fails to send keeps a
Retry button, and a note between messages says when someone joins or leaves the
conversation. Image attachments show as pictures under their message, fetched
when the message comes into view. An invitation shows as a card with its time,
place and organizer, a Join button for Teams, Meet, Zoom and Webex links, and
Add to Calendar, which hands the invitation to the calendar application; answers
to it read as a line such as "Ana accepted". The right-click menu also reacts
to a message with 👍 👎 ❤ 😂 ‼ or ❓, shown at once and sent after five seconds
unless undone; a reaction is a short reply with an X-Zirbe-Reaction header, the
same as the iPhone app sends, so each app shows the other's reactions as
badges, and Gmail's reactions show the same way. In Settings each account
can have a label and one of eight colors, shown as a dot beside its
conversations in All Inboxes; an account whose server starts refusing the saved
password says so there and takes a new one with Sign In Again; and the list can
show none to three lines of each conversation's latest message. Block, in a
conversation's menu, moves that sender's mail to Junk now and at every check
for mail after, until the sender is unblocked in Settings; Trash All from the
same menu moves every conversation in the folder shown whose mail comes only
from that sender to Trash. If the copy of a sent message cannot be
saved to the server's Sent folder, the status line says so and Zirbe keeps
trying at every check for mail. Mail you sent
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

Messages are shown as text. A message with an HTML version carries
Web View and Show Images at the top of its bubble. Web View downloads that
message again from your server and shows its full HTML in the reading pane,
drawn by Zirbe itself with nothing loaded from the network; pictures the sender
embedded in the message still show. Show Images loads the sender's linked
pictures too, which can tell the sender you opened the message and roughly
where you are. Settings can open HTML email in Web View and load its pictures
without being asked; both are off until you turn them on. Links open in your
browser. Apart from those images, the only connections are to your own IMAP and
SMTP servers. [SECURITY.md](SECURITY.md#what-zirbe-can-access) says what else
Zirbe reads, and [What Zirbe stores](SECURITY.md#what-zirbe-stores) where it
keeps your mail, settings and password.

## License

MIT. See [LICENSE](LICENSE).
