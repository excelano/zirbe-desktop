// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::time::Duration;

use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{AsyncSmtpConnection, Tls, TlsParametersBuilder};
use lettre::transport::smtp::commands::Ehlo;
use lettre::transport::smtp::extension::ClientId;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::model::Account;
use crate::owed::NotSent;

fn transport(
    account: &Account,
    password: &str,
) -> Result<AsyncSmtpTransport<Tokio1Executor>, String> {
    let host = account.smtp_host.trim();
    let parameters = TlsParametersBuilder::new(host.to_string())
        .build_rustls()
        .map_err(|error| format!("{host}: {error}"))?;
    let tls = if account.smtp_port == 465 {
        Tls::Wrapper(parameters)
    } else {
        Tls::Required(parameters)
    };
    Ok(
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
            .port(account.smtp_port)
            .tls(tls)
            .credentials(Credentials::new(account.key(), password.to_string()))
            .timeout(Some(Duration::from_secs(30)))
            .build(),
    )
}

pub(crate) async fn send(
    account: &Account,
    password: &str,
    from: Option<&str>,
    to: &[String],
    raw: &[u8],
) -> Result<(), NotSent> {
    let host = account.smtp_host.trim();
    let address = |text: &str| {
        text.parse::<lettre::Address>()
            .map_err(|error| NotSent::Never(format!("{text} is not an address: {error}")))
    };
    let envelope = lettre::address::Envelope::new(
        from.map(address).transpose()?,
        to.iter()
            .map(|text| address(text))
            .collect::<Result<_, _>>()?,
    )
    .map_err(|error| NotSent::Never(format!("the message has no recipients: {error}")))?;
    transport(account, password)
        .map_err(NotSent::Later)?
        .send_raw(&envelope, raw)
        .await
        .map(|_| ())
        .map_err(|error| {
            if error.is_permanent() {
                NotSent::Never(format!("{host} refused the message: {error}"))
            } else {
                NotSent::Later(format!("{host}: {error}"))
            }
        })
}

pub(crate) async fn size_limit(account: &Account) -> Option<u64> {
    let host = account.smtp_host.trim();
    let parameters = TlsParametersBuilder::new(host.to_string())
        .build_rustls()
        .ok()?;
    let client = ClientId::default();
    let wrapped = account.smtp_port == 465;
    let mut connection = AsyncSmtpConnection::connect_tokio1(
        (host, account.smtp_port),
        Some(Duration::from_secs(30)),
        &client,
        wrapped.then(|| parameters.clone()),
        None,
    )
    .await
    .ok()?;
    if !wrapped {
        connection.starttls(parameters, &client).await.ok()?;
    }
    let reply = connection.command(Ehlo::new(client)).await.ok();
    let _ = connection.quit().await;
    reply?.message().find_map(announced_size)
}

fn announced_size(line: &str) -> Option<u64> {
    let mut words = line.split_whitespace();
    words
        .next()
        .filter(|word| word.eq_ignore_ascii_case("SIZE"))?;
    words.next()?.parse().ok().filter(|limit| *limit > 0)
}

pub(crate) async fn check(account: &Account, password: &str) -> Result<(), String> {
    let host = account.smtp_host.trim();
    match transport(account, password)?.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("{host} did not accept a connection")),
        Err(error) => Err(format!("{host}: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::announced_size;

    #[test]
    fn reads_the_size_a_server_announces() {
        assert_eq!(announced_size("SIZE 52428800"), Some(52_428_800));
        assert_eq!(announced_size("size 1000"), Some(1000));
        assert_eq!(announced_size("SIZE"), None);
        assert_eq!(announced_size("SIZE 0"), None);
        assert_eq!(announced_size("8BITMIME"), None);
    }
}
