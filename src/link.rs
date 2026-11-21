// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use url::{Host, Url};

const TRACKING_PARAMETERS: &[&str] = &[
    "fbclid",
    "gclid",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "igshid",
    "mc_cid",
    "mc_eid",
    "_hsenc",
    "_hsmi",
    "mkt_tok",
    "oly_anon_id",
    "oly_enc_id",
    "vero_id",
];

pub struct Check {
    pub address: String,
    pub host: String,
    pub warnings: Vec<String>,
    pub cleaned: Option<String>,
}

pub fn check(href: &str, text: &str) -> Option<Check> {
    let url = Url::parse(href.trim()).ok()?;
    let host = url.host_str()?.to_string();
    let mut warnings = Vec::new();
    if matches!(url.host(), Some(Host::Ipv4(_) | Host::Ipv6(_))) {
        warnings.push(format!(
            "This link goes to a numeric address, {host}, not a named site."
        ));
    }
    if host.split('.').any(|label| label.starts_with("xn--")) {
        let (shown, _) = idna::domain_to_unicode(&host);
        warnings.push(format!(
            "The site's name is written with international letters: it reads {shown} but is {host}. Letters that look alike are a common trick."
        ));
    }
    if let Some(named) = named_host(text)
        && !same_site(&named, &host)
    {
        warnings.push(format!("The link reads {named} but goes to {host}."));
    }
    let cleaned = clean(&url)
        .map(String::from)
        .filter(|cleaned| cleaned != url.as_str());
    Some(Check {
        address: url.to_string(),
        host,
        warnings,
        cleaned,
    })
}

fn named_host(text: &str) -> Option<String> {
    let text = text.trim().trim_end_matches(['.', ',', ';', ':', ')']);
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let url = if text.contains("://") {
        Url::parse(text).ok()?
    } else {
        Url::parse(&format!("https://{text}")).ok()?
    };
    let host = url.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    let (_, top) = host.rsplit_once('.')?;
    (top.len() >= 2 && top.chars().all(|letter| letter.is_ascii_alphabetic())
        || top.starts_with("xn--"))
    .then_some(host)
}

fn same_site(named: &str, host: &str) -> bool {
    let bare = |host: &str| {
        let host = host.to_ascii_lowercase();
        host.strip_prefix("www.")
            .map(str::to_string)
            .unwrap_or(host)
    };
    let (named, host) = (bare(named), bare(host));
    host == named || host.ends_with(&format!(".{named}"))
}

fn clean(url: &Url) -> Option<Url> {
    let host = url.host_str()?.to_ascii_lowercase();
    let wrapped = if host.ends_with("safelinks.protection.outlook.com") {
        parameter(url, "url")
    } else if host == "www.google.com" && url.path() == "/url" {
        parameter(url, "q").or_else(|| parameter(url, "url"))
    } else {
        None
    };
    if let Some(inner) = wrapped.and_then(|inner| Url::parse(&inner).ok())
        && matches!(inner.scheme(), "https" | "http")
    {
        return clean(&inner).or(Some(inner));
    }
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| {
            let name = name.to_ascii_lowercase();
            !name.starts_with("utm_") && !TRACKING_PARAMETERS.contains(&name.as_str())
        })
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    let mut cleaned = url.clone();
    if kept.is_empty() {
        cleaned.set_query(None);
    } else {
        cleaned.query_pairs_mut().clear().extend_pairs(&kept);
    }
    Some(cleaned)
}

fn parameter(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

#[cfg(test)]
mod tests {
    use super::check;

    #[test]
    fn an_ordinary_link_carries_no_warning() {
        let checked = check("https://www.example.com/account", "example.com").expect("a link");
        assert_eq!(checked.host, "www.example.com");
        assert!(checked.warnings.is_empty());
        assert_eq!(checked.cleaned, None);
        assert!(
            check("https://news.example.com/a", "Read the story")
                .is_some_and(|checked| checked.warnings.is_empty())
        );
    }

    #[test]
    fn link_text_naming_another_site_is_flagged() {
        let checked =
            check("https://login.example.net/", "https://www.mybank.com/login").expect("a link");
        assert_eq!(
            checked.warnings,
            ["The link reads www.mybank.com but goes to login.example.net."]
        );
        assert!(
            check("https://click.mybank.com/x", "mybank.com.")
                .is_some_and(|checked| checked.warnings.is_empty())
        );
        assert!(
            check("https://example.com/", "here.")
                .is_some_and(|checked| checked.warnings.is_empty())
        );
    }

    #[test]
    fn numeric_and_international_hosts_are_flagged() {
        let numeric = check("http://3232235777/login", "").expect("a link");
        assert_eq!(numeric.host, "192.168.1.1");
        assert_eq!(numeric.warnings.len(), 1);
        let lookalike = check("https://аpple.com/", "").expect("a link");
        assert_eq!(lookalike.host, "xn--pple-43d.com");
        assert!(lookalike.warnings[0].contains("аpple.com"));
    }

    #[test]
    fn tracking_is_cleaned_away() {
        let tagged = check(
            "https://shop.example/item?id=7&utm_source=mail&fbclid=abc",
            "",
        )
        .expect("a link");
        assert_eq!(
            tagged.cleaned.as_deref(),
            Some("https://shop.example/item?id=7")
        );
        let wrapped = check(
            "https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fexample.org%2Fdoc%3Futm_medium%3Demail&data=x",
            "",
        )
        .expect("a link");
        assert_eq!(wrapped.cleaned.as_deref(), Some("https://example.org/doc"));
    }
}
