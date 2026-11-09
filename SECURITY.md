# Security Policy

## Reporting a vulnerability

Please report suspected vulnerabilities privately through GitHub Security
Advisories at https://github.com/excelano/zirbe-desktop/security/advisories/new.
If you would rather not use GitHub, email david.anderson@excelano.com instead. I
aim to respond within seven days.

Please do not open public issues for security problems.

## Supported versions

The latest 0.x release receives security fixes. Older versions are not
supported.

## What this application is

Zirbe reads mail from the IMAP account a user signs in to and sends mail through
that account's SMTP server. It connects to those two servers and to nothing else.
Both connections are encrypted with TLS against the system's trusted
certificates: implicit TLS on ports 993 and 465, and STARTTLS required on any
other port, so a password is never sent unencrypted. There is no telemetry, no
update check and no account of its own.

Mail arrives from other people and is the input this application has to
withstand. Messages are parsed by the `mail-parser` crate and shown as plain
text, with HTML turned into text by `html2text`. A message's HTML is rendered
only when the user asks for Email View, in a separate process running the
system web view (WebKitGTK on Linux) under a Content Security Policy that allows
inline styles and images embedded in the message and nothing else: no script,
frame, stylesheet, font or request to any server. Script markup is also turned
off in the engine itself. Show Images relaxes the policy for images alone, which
lets the sender's servers see the request. Links open in the system browser and
never in that window. A crash or hang on a malformed or hostile message, or
anything in Email View that reaches the network or runs script, is worth
reporting.

## What it stores

Each account's password, in the system keyring (the Secret Service on Linux),
never on disk by Zirbe itself. The account's address, name and servers, in
`~/.config/zirbe/accounts.json`. The account's folder list, and the newest
messages of its inbox, its Sent folder and each folder the user has opened, in
`~/.cache/zirbe`, written readable by their owner only.

## Verifying releases

Every release lists a SHA-256 for each artefact. Verify a download before
running it.
