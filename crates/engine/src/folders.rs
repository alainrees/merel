//! Folders a person makes, renames and removes: `folder.create`, `folder.rename` and
//! `folder.delete`. The sample account changes at once; a real account hands the server the
//! job and takes the folder list the server answers with (`Event::Folders`), so what the pane
//! shows is always what the server has. Only ordinary folders can be renamed or removed: the
//! inbox and the other special folders belong to the account.

use mailcraft_model::{Folder, FolderKind, clean_text};
use serde_json::Value;

use crate::commands::{CommandError, str_param};
use crate::{Job, SAMPLE_ACCOUNT, Session};

/// The longest folder name accepted, in characters.
pub const MAX_FOLDER_NAME: usize = 80;

/// The folder name as it goes to the server: trimmed, cleaned, without the separators every
/// server reads as a path, and not empty.
pub fn clean_folder_name(name: &str) -> Result<String, CommandError> {
    let name = clean_text(name).trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_FOLDER_NAME || name.contains(['/', '\\']) {
        return Err(CommandError::Param("name"));
    }
    Ok(name)
}

/// `{ name, parent?: folder id, account_id? }`: a folder under `parent`, or at the top of the
/// account (`account_id`, else the current one) when there is no parent.
pub fn create(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let name = clean_folder_name(str_param(params, "name")?)?;
    let parent = match params.get("parent").and_then(Value::as_str) {
        Some(id) => Some(s.folder(id).cloned().ok_or(CommandError::Param("parent"))?),
        None => None,
    };
    let account_id = match (&parent, params.get("account_id").and_then(Value::as_str)) {
        (Some(p), _) => p.account_id.clone(),
        (None, Some(id)) => s.accounts.iter().find(|a| a.id == id).map(|a| a.id.clone()).ok_or(CommandError::Param("account_id"))?,
        (None, None) => s.current_account().map(|a| a.id.clone()).ok_or(CommandError::NoSelection)?,
    };
    if account_id != SAMPLE_ACCOUNT {
        s.pending.push(Job::CreateFolder { account_id, parent: parent.map(|p| p.path), name });
        return Ok(());
    }
    let path = match &parent {
        Some(p) => format!("{}/{name}", p.path),
        None => name.clone(),
    };
    let id = format!("{account_id}/{path}");
    if s.folder(&id).is_some() || s.folders.iter().any(|f| f.account_id == account_id && f.name.eq_ignore_ascii_case(&name)) {
        return Err(CommandError::Param("name"));
    }
    s.folders.push(Folder { id, account_id, name, path, kind: FolderKind::Other, unread: 0 });
    sort(s);
    Ok(())
}

/// `{ id, name }`: an ordinary folder gets a new name where it is.
pub fn rename(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let name = clean_folder_name(str_param(params, "name")?)?;
    let folder = ordinary(s, params)?;
    if folder.account_id != SAMPLE_ACCOUNT {
        s.pending.push(Job::RenameFolder { account_id: folder.account_id, path: folder.path, name });
        return Ok(());
    }
    let path = match folder.path.rsplit_once('/') {
        Some((parent, _)) => format!("{parent}/{name}"),
        None => name.clone(),
    };
    let id = format!("{}/{path}", folder.account_id);
    if id != folder.id && s.folder(&id).is_some() {
        return Err(CommandError::Param("name"));
    }
    for m in s.messages.iter_mut().filter(|m| m.folder_id == folder.id) {
        m.folder_id = id.clone();
    }
    if s.selected_folder.as_deref() == Some(&folder.id) {
        s.selected_folder = Some(id.clone());
    }
    if let Some(f) = s.folders.iter_mut().find(|f| f.id == folder.id) {
        f.id = id;
        f.path = path;
        f.name = name;
    }
    sort(s);
    Ok(())
}

/// `{ id }`: an ordinary folder goes, with the messages in it.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let folder = ordinary(s, params)?;
    if folder.account_id != SAMPLE_ACCOUNT {
        s.pending.push(Job::DeleteFolder { account_id: folder.account_id, path: folder.path });
        return Ok(());
    }
    s.folders.retain(|f| f.id != folder.id);
    s.messages.retain(|m| m.folder_id != folder.id);
    if s.selected_folder.as_deref() == Some(&folder.id) {
        s.selected_folder = s.folders.iter().find(|f| f.account_id == folder.account_id && f.kind == FolderKind::Inbox).map(|f| f.id.clone());
        s.selected_message = None;
    }
    Ok(())
}

/// The folder `id` names, when it is one a person may rename or remove.
fn ordinary(s: &Session, params: &Value) -> Result<Folder, CommandError> {
    let folder = s.folder(str_param(params, "id")?).cloned().ok_or(CommandError::NotFound)?;
    if folder.kind != FolderKind::Other {
        return Err(CommandError::Param("id"));
    }
    Ok(folder)
}

fn sort(s: &mut Session) {
    s.folders.sort_by(|a, b| {
        a.account_id
            .cmp(&b.account_id)
            .then_with(|| a.kind.rank().cmp(&b.kind.rank()))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_sample_folder_is_made_renamed_and_removed_in_place() {
        let mut s = Session::sample();
        let inbox = s.selected_folder.clone().expect("inbox");
        s.run("folder.create", &json!({ "name": "  Projecten ", "parent": inbox })).expect("create");
        let id = "sample/INBOX/Projecten";
        assert_eq!(s.folder(id).map(|f| (f.name.as_str(), f.kind)), Some(("Projecten", FolderKind::Other)));
        assert!(matches!(s.run("folder.create", &json!({ "name": "projecten", "parent": inbox })), Err(CommandError::Param("name"))), "no twins");
        assert!(matches!(s.run("folder.create", &json!({ "name": "a/b" })), Err(CommandError::Param("name"))));
        assert!(matches!(s.run("folder.rename", &json!({ "id": inbox, "name": "Anders" })), Err(CommandError::Param("id"))), "special folders stay");

        s.run("folder.select", &json!({ "id": id })).expect("select");
        s.run("folder.rename", &json!({ "id": id, "name": "Klanten" })).expect("rename");
        assert!(s.folder(id).is_none());
        assert_eq!(s.selected_folder.as_deref(), Some("sample/INBOX/Klanten"));
        assert_eq!(s.folder("sample/INBOX/Klanten").map(|f| f.path.as_str()), Some("INBOX/Klanten"));

        s.run("folder.delete", &json!({ "id": "sample/INBOX/Klanten" })).expect("delete");
        assert!(s.folder("sample/INBOX/Klanten").is_none());
        assert_eq!(s.selected_folder.as_deref(), Some(inbox.as_str()), "back to the inbox");
        assert!(s.take_jobs().is_empty(), "the sample account has no server");
    }

    #[test]
    fn a_real_account_hands_the_server_the_job_and_takes_its_folder_list() {
        use mailcraft_mailbox::RemoteFolder;

        let mut s = Session::with_accounts(vec![crate::tests::account("acc")]);
        let listed = |names: &[&str]| -> Vec<RemoteFolder> {
            let mut out = vec![RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true }];
            out.extend(names.iter().map(|n| RemoteFolder {
                path: format!("INBOX/{n}"),
                name: (*n).into(),
                kind: FolderKind::Other,
                selectable: true,
            }));
            out
        };
        s.apply(crate::Event::Connected { account_id: "acc".into(), folders: listed(&[]), syncs_items: false });
        s.apply(crate::Event::Headers { folder_id: "acc/INBOX".into(), headers: Vec::new(), offset: 0, limit: crate::HEADER_LIMIT });
        s.take_jobs();
        s.run("folder.create", &json!({ "name": "Projecten", "parent": "acc/INBOX" })).expect("create");
        assert_eq!(s.take_jobs(), vec![Job::CreateFolder { account_id: "acc".into(), parent: Some("INBOX".into()), name: "Projecten".into() }]);
        assert!(s.folder("acc/INBOX/Projecten").is_none(), "the server decides");
        s.apply(crate::Event::Folders { account_id: "acc".into(), folders: listed(&["Projecten"]) });
        assert_eq!(s.folder("acc/INBOX/Projecten").map(|f| f.name.as_str()), Some("Projecten"));

        s.run("folder.select", &json!({ "id": "acc/INBOX/Projecten" })).expect("select");
        s.take_jobs();
        s.run("folder.rename", &json!({ "id": "acc/INBOX/Projecten", "name": "Klanten" })).expect("rename");
        assert_eq!(s.take_jobs(), vec![Job::RenameFolder { account_id: "acc".into(), path: "INBOX/Projecten".into(), name: "Klanten".into() }]);
        s.apply(crate::Event::Folders { account_id: "acc".into(), folders: listed(&["Klanten"]) });
        assert_eq!(s.selected_folder.as_deref(), Some("acc/INBOX"), "the renamed folder was selected: the inbox takes over");
        assert!(matches!(&s.take_jobs()[..], [Job::Headers { folder_id, .. }] if folder_id == "acc/INBOX"));

        s.run("folder.delete", &json!({ "id": "acc/INBOX/Klanten" })).expect("delete");
        assert_eq!(s.take_jobs(), vec![Job::DeleteFolder { account_id: "acc".into(), path: "INBOX/Klanten".into() }]);
    }
}
