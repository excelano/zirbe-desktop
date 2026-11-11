# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately, using the **Report a vulnerability** button on the Security tab of this repository. If that button is not there, or you would rather not go through GitHub, email david.anderson@excelano.com instead. I aim to respond within seven days.

Please do not open public issues for security problems.

## Supported versions

The latest release receives security fixes. Older versions are not supported. A fix ships in a new release; update the way you installed.

## What Zirbe can access

Zirbe reads mail from the IMAP account a user signs in to and sends mail through
that account's SMTP server. It connects to those two servers and to nothing else.
Both connections are encrypted with TLS against the system's trusted
certificates: implicit TLS on ports 993 and 465, and STARTTLS required on any
other port, so a password is never sent unencrypted. There is no telemetry, no
update check and no account of its own.

Mail arrives from other people and is the input this application has to
withstand. Messages are parsed by the `mail-parser` crate and shown as plain
text, with HTML turned into text by `html2text`. A message's HTML is rendered
only when the user asks for Web View, by Blitz, an HTML and CSS engine built
into Zirbe that has no script engine. Blitz fetches nothing itself: every
resource a page asks for goes through Zirbe's own provider, which answers only
the images embedded in the message and gives everything else an empty answer.
Show Images lets that provider fetch the message's `<img>` sources over HTTPS or
HTTP, which lets the sender's servers see the request; it still requests no
stylesheet, font, frame or background image, and uses a response only if it is
an image. Links open in the system browser. A crash or hang on a malformed or
hostile message, or anything in Web View that reaches the network beyond
those images, is worth reporting.

On the session bus, Zirbe reads the address book that GNOME Contacts and
Evolution keep (Evolution Data Server) for recipient suggestions and sender
photos, and never writes to it; a photo is shown only when it is in the contact
card or in a file on this computer, never fetched from a URL. It also posts
new-mail notifications and the launcher's unread count there.

## What Zirbe stores

Each account's password, in the system keyring (the Secret Service on Linux),
never on disk by Zirbe itself. The account's address, name, label, color and
servers, in `~/.config/zirbe/accounts.json`, and the display and notification
settings in `~/.config/zirbe/settings.json`. The account's folder list, its
blocked senders, and the newest messages of its inbox, its Sent folder and each
folder the user has opened, in `~/.cache/zirbe`, written readable by their
owner only. An attachment the user opens is written to a private folder under
`$XDG_RUNTIME_DIR/zirbe`, which Zirbe empties each time it starts and the
system clears at logout.

## Verifying releases

Every release lists a SHA-256 for each artifact. Verify a download before
running it.
