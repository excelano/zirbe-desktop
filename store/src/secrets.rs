// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use keyring_core::{Entry, Error};

const SERVICE: &str = "zirbe";

pub(crate) fn open() -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let store = zbus_secret_service_keyring_store::Store::new();
    #[cfg(target_os = "macos")]
    let store = apple_native_keyring_store::keychain::Store::new();
    #[cfg(target_os = "windows")]
    let store = windows_native_keyring_store::Store::new();
    keyring_core::set_default_store(
        store.map_err(|error| format!("the keyring could not be opened: {error}"))?,
    );
    Ok(())
}

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account).map_err(|error| format!("the keyring could not be used: {error}"))
}

pub(crate) fn password(account: &str) -> Result<String, String> {
    match entry(account)?.get_password() {
        Ok(password) => Ok(password),
        Err(Error::NoEntry) => Err(format!("no password is saved for {account}")),
        Err(error) => Err(format!("the keyring could not be read: {error}")),
    }
}

pub(crate) fn save(account: &str, password: &str) -> Result<(), String> {
    entry(account)?
        .set_password(password)
        .map_err(|error| format!("the password could not be saved: {error}"))
}

pub(crate) fn forget(account: &str) -> Result<(), String> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(Error::NoEntry) => Ok(()),
        Err(error) => Err(format!("the password could not be removed: {error}")),
    }
}
