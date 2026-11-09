// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;

use crate::model::{FolderInfo, INBOX, Role};

const NAMED: [(Role, &[&str]); 5] = [
    (Role::Drafts, &["Drafts", "Draft"]),
    (
        Role::Sent,
        &["Sent", "Sent Items", "Sent Messages", "Sent Mail"],
    ),
    (Role::Archive, &["Archive", "Archives"]),
    (
        Role::Junk,
        &["Junk", "Spam", "Junk E-mail", "Junk Email", "Bulk Mail"],
    ),
    (
        Role::Trash,
        &["Trash", "Deleted Items", "Deleted Messages", "Bin"],
    ),
];

pub(crate) struct Listed {
    pub name: String,
    pub delimiter: Option<String>,
    pub selectable: bool,
    pub role: Option<Role>,
}

pub(crate) fn arrange(listed: Vec<Listed>) -> Vec<FolderInfo> {
    let mut folders: Vec<(Vec<String>, FolderInfo)> = listed
        .iter()
        .map(|folder| {
            let path = path(&folder.name, folder.delimiter.as_deref());
            let inbox = folder.name.eq_ignore_ascii_case(INBOX);
            let info = FolderInfo {
                name: folder.name.clone(),
                label: if inbox {
                    "Inbox".into()
                } else {
                    path.last().cloned().unwrap_or_default()
                },
                depth: path.len().saturating_sub(1),
                role: if inbox {
                    Some(Role::Inbox)
                } else {
                    folder.role
                },
                selectable: folder.selectable,
                unread: 0,
            };
            (path, info)
        })
        .collect();
    for (role, names) in NAMED {
        if folders.iter().any(|(_, info)| info.role == Some(role)) {
            continue;
        }
        let found = names.iter().find_map(|wanted| {
            folders.iter().position(|(_, info)| {
                info.role.is_none() && info.selectable && info.label.eq_ignore_ascii_case(wanted)
            })
        });
        if let Some(index) = found {
            folders[index].1.role = Some(role);
        }
    }
    let ranks: Vec<(Vec<String>, u8)> = folders
        .iter()
        .map(|(path, info)| (path.clone(), info.role.map_or(u8::MAX, |role| role as u8)))
        .collect();
    let rank = |prefix: &[String]| {
        ranks
            .iter()
            .find(|(path, _)| path.as_slice() == prefix)
            .map_or(u8::MAX, |(_, rank)| *rank)
    };
    folders.sort_by_cached_key(|(path, _)| {
        (1..=path.len())
            .map(|end| (rank(&path[..end]), path[end - 1].to_lowercase()))
            .collect::<Vec<_>>()
    });
    folders.into_iter().map(|(_, info)| info).collect()
}

fn path(name: &str, delimiter: Option<&str>) -> Vec<String> {
    let decoded = decode(name);
    let Some(delimiter) = delimiter.filter(|delimiter| !delimiter.is_empty()) else {
        return vec![decoded];
    };
    let prefix = format!("{INBOX}{delimiter}");
    let inside = match decoded.get(..prefix.len()) {
        Some(start) if decoded.len() > prefix.len() && start.eq_ignore_ascii_case(&prefix) => {
            &decoded[prefix.len()..]
        }
        _ => &decoded,
    };
    inside.split(delimiter).map(str::to_string).collect()
}

fn decode(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let shifted = &rest[start + 1..];
        let Some(end) = shifted.find('-') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let encoded = &shifted[..end];
        if encoded.is_empty() {
            out.push('&');
        } else {
            match STANDARD_NO_PAD.decode(encoded.replace(',', "/")) {
                Ok(bytes) => {
                    let units: Vec<u16> = bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| u16::from_be_bytes(*pair))
                        .collect();
                    out.push_str(&String::from_utf16_lossy(&units));
                }
                Err(_) => out.push_str(&rest[start..start + end + 2]),
            }
        }
        rest = &shifted[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(name: &str, role: Option<Role>) -> Listed {
        Listed {
            name: name.into(),
            delimiter: Some(".".into()),
            selectable: true,
            role,
        }
    }

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode("&ZeVnLIqe-"), "日本語");
    }

    #[test]
    fn puts_the_inbox_and_roles_first_and_keeps_children_under_parents() {
        let folders = arrange(vec![
            listed("INBOX.Projects.Zirbe", None),
            listed("INBOX.Trash", None),
            listed("INBOX.Projects", None),
            listed("INBOX.Sent Messages", Some(Role::Sent)),
            listed("INBOX.Sent", None),
            listed("INBOX", None),
            listed("INBOX.Archive", None),
        ]);
        let shown: Vec<(&str, usize, Option<Role>)> = folders
            .iter()
            .map(|folder| (folder.label.as_str(), folder.depth, folder.role))
            .collect();
        assert_eq!(
            shown,
            [
                ("Inbox", 0, Some(Role::Inbox)),
                ("Sent Messages", 0, Some(Role::Sent)),
                ("Archive", 0, Some(Role::Archive)),
                ("Trash", 0, Some(Role::Trash)),
                ("Projects", 0, None),
                ("Zirbe", 1, None),
                ("Sent", 0, None),
            ]
        );
    }
}
