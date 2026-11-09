// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

pub struct Preset {
    pub provider: Option<&'static str>,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub hint: Option<&'static str>,
}

struct Known {
    provider: &'static str,
    imap: &'static str,
    smtp: &'static str,
    hint: &'static str,
}

const ICLOUD: Known = Known {
    provider: "iCloud",
    imap: "imap.mail.me.com",
    smtp: "smtp.mail.me.com",
    hint: "iCloud needs an app-specific password. Make one at account.apple.com under Sign-In and Security.",
};
const FASTMAIL: Known = Known {
    provider: "Fastmail",
    imap: "imap.fastmail.com",
    smtp: "smtp.fastmail.com",
    hint: "Fastmail needs an app password. Make one in Settings, Privacy & Security, App passwords.",
};
const GMAIL: Known = Known {
    provider: "Gmail",
    imap: "imap.gmail.com",
    smtp: "smtp.gmail.com",
    hint: "Gmail needs an app password, which needs 2-Step Verification on the account.",
};
const YAHOO: Known = Known {
    provider: "Yahoo",
    imap: "imap.mail.yahoo.com",
    smtp: "smtp.mail.yahoo.com",
    hint: "Yahoo needs an app password. Make one in Account Security, Generate app password.",
};
const AOL: Known = Known {
    provider: "AOL",
    imap: "imap.aol.com",
    smtp: "smtp.aol.com",
    hint: "AOL needs an app password. Make one in Account Security, Generate app password.",
};
const OUTLOOK: Known = Known {
    provider: "Outlook",
    imap: "outlook.office365.com",
    smtp: "smtp.office365.com",
    hint: "Most Microsoft accounts no longer accept a password for mail, and this version cannot sign in with Microsoft yet.",
};

fn known(domain: &str) -> Option<&'static Known> {
    Some(match domain {
        "icloud.com" | "me.com" | "mac.com" => &ICLOUD,
        "fastmail.com" => &FASTMAIL,
        "gmail.com" | "googlemail.com" => &GMAIL,
        "yahoo.com" | "ymail.com" | "rocketmail.com" => &YAHOO,
        "aol.com" => &AOL,
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => &OUTLOOK,
        _ => return None,
    })
}

pub fn for_address(address: &str) -> Option<Preset> {
    let (local, domain) = address.trim().rsplit_once('@')?;
    let domain = domain.to_ascii_lowercase();
    if local.is_empty() || !domain.contains('.') {
        return None;
    }
    Some(match known(&domain) {
        Some(known) => Preset {
            provider: Some(known.provider),
            imap_host: known.imap.to_string(),
            imap_port: 993,
            smtp_host: known.smtp.to_string(),
            smtp_port: 587,
            hint: Some(known.hint),
        },
        None => Preset {
            provider: None,
            imap_host: format!("imap.{domain}"),
            imap_port: 993,
            smtp_host: format!("smtp.{domain}"),
            smtp_port: 587,
            hint: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::for_address;

    #[test]
    fn knows_the_big_providers_and_guesses_the_rest() {
        let icloud = for_address("Someone@ME.com").expect("a preset");
        assert_eq!(icloud.provider, Some("iCloud"));
        assert_eq!(icloud.imap_host, "imap.mail.me.com");
        let other = for_address("ana@example.org").expect("a guess");
        assert_eq!(other.provider, None);
        assert_eq!(other.smtp_host, "smtp.example.org");
        assert!(for_address("nobody").is_none());
        assert!(for_address("@example.org").is_none());
    }
}
