// Signs in with the address and password given, syncs, prints the
// conversations, and signs out again, so nothing is left in the keyring.
//
//     ZIRBE_PASSWORD=... cargo run --example sync -p zirbe-store -- <address> [imap-host smtp-host]
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::time::Duration;

use zirbe_store::{Account, MailSource, Status, Store};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let Some(address) = arguments.first() else {
        eprintln!("usage: sync <address> [imap-host smtp-host], with ZIRBE_PASSWORD set");
        std::process::exit(2);
    };
    let password = std::env::var("ZIRBE_PASSWORD").unwrap_or_default();
    let preset = zirbe_store::for_address(address).expect("an email address");
    let account = Account {
        address: address.clone(),
        name: String::new(),
        imap_host: arguments.get(1).cloned().unwrap_or(preset.imap_host),
        imap_port: preset.imap_port,
        smtp_host: arguments.get(2).cloned().unwrap_or(preset.smtp_host),
        smtp_port: preset.smtp_port,
    };
    let scratch = std::env::temp_dir().join(format!("zirbe-sync-{}", std::process::id()));
    let store = Store::open(scratch.join("config"), scratch.join("cache"), || {});
    match store.sign_in(account.clone(), password).recv() {
        Ok(Ok(())) => println!("signed in"),
        Ok(Err(error)) => {
            println!("sign-in refused: {error}");
            return;
        }
        Err(_) => return,
    }
    loop {
        std::thread::sleep(Duration::from_millis(500));
        match store.status() {
            Status::Syncing => continue,
            Status::Failed(error) => println!("sync failed: {error}"),
            _ => {}
        }
        break;
    }
    for conversation in store.conversations().iter().take(20) {
        println!(
            "{:>3}  {}  ({} unread)",
            conversation.messages.len(),
            conversation.title,
            conversation.unread
        );
    }
    store.sign_out(&account);
    std::thread::sleep(Duration::from_secs(1));
    let _ = std::fs::remove_dir_all(scratch);
}
