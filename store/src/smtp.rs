// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::time::Duration;

use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParametersBuilder};
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

use crate::model::Account;

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
    message: lettre::Message,
) -> Result<(), String> {
    let host = account.smtp_host.trim();
    transport(account, password)?
        .send(message)
        .await
        .map(|_| ())
        .map_err(|error| {
            if error.is_permanent() {
                format!("{host} refused the message: {error}")
            } else {
                format!("{host}: {error}")
            }
        })
}

pub(crate) async fn check(account: &Account, password: &str) -> Result<(), String> {
    let host = account.smtp_host.trim();
    match transport(account, password)?.test_connection().await {
        Ok(true) => Ok(()),
        Ok(false) => Err(format!("{host} did not accept a connection")),
        Err(error) => Err(format!("{host}: {error}")),
    }
}
