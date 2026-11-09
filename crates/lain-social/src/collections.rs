//! `lain.social.collections@1`: collections, their sharing, and work
//! shares.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use lain_api::{CollectionPatch, notify, visibility};
use lain_core::Error;
use lain_store::Bucket;

use crate::graph::{are_friends, blocked_either, can_see_tx, level_allows};
use crate::{
    Collection, CollectionItem, Hidden, Rx, SUBJECT_PROFILE, Social, Tx, WorkRef, clean_text, forbidden, get_json,
    hidden_work, input, invalid, need_user, normalize_work, not_found, out, scan, valid_visibility, work_key,
};

/// Ops of `lain.social.collections@1`.
pub mod collections_op {
    /// `{viewer_id, owner_id?, hidden_libraries?}` → `[Collection]`
    pub const LIST: &str = "list";
    /// `{viewer_id, id, hidden_libraries?}` → `Collection`
    pub const GET: &str = "get";
    /// `{user_id, name, description?, visibility?}` → `Collection`
    pub const CREATE: &str = "create";
    /// `{user_id, id, name?, description?, visibility?, hidden_libraries?}` → `Collection`
    pub const UPDATE: &str = "update";
    /// `{user_id, id}` → true
    pub const DELETE: &str = "delete";
    /// `{user_id, id, work, note?, hidden_libraries?}` → `Collection`
    pub const ADD_ITEM: &str = "add-item";
    /// `{user_id, id, work, hidden_libraries?}` → `Collection`
    pub const REMOVE_ITEM: &str = "remove-item";
    /// `{user_id, id, to, message?}` → `{sent}`
    pub const SHARE: &str = "share";
    /// `{user_id, id, other_id, hidden_libraries?}` → `Collection`
    pub const UNSHARE: &str = "unshare";
    /// `{user_id, to, work, message?}` → `{sent}`
    pub const SHARE_WORK: &str = "share-work";
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ListCollectionsInput {
    #[serde(default)]
    pub viewer_id: String,
    /// Empty: the viewer's own collections plus those shared with them;
    /// otherwise the owner's collections the viewer may open.
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GetCollectionInput {
    #[serde(default)]
    pub viewer_id: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreateCollectionInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub visibility: String,
}

/// Any change to a collection the user owns; the answer is the stored
/// collection with works of hidden libraries left out (D-156).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChangeCollectionInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub id: String,
    #[serde(default, flatten)]
    pub patch: CollectionPatch,
    #[serde(default)]
    pub work: Option<WorkRef>,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub other_id: String,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ShareWorkInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default)]
    pub message: String,
}

/// Who received a share.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareOutput {
    pub sent: Vec<String>,
}

/// The owner always; nobody across a block; explicit shares while still
/// friends; then the collection's own visibility.
fn can_open(rx: &impl Rx, viewer: &str, c: &Collection) -> Result<bool, Error> {
    if viewer == c.owner_id {
        return Ok(true);
    }
    if blocked_either(rx, viewer, &c.owner_id)? {
        return Ok(false);
    }
    if are_friends(rx, viewer, &c.owner_id)? && c.shared_with.iter().any(|id| id == viewer) {
        return Ok(true);
    }
    level_allows(rx, &c.visibility, viewer, &c.owner_id)
}

fn get_collection(rx: &impl Rx, id: &str) -> Result<Collection, Error> {
    if id.is_empty() {
        return Err(not_found("unknown collection"));
    }
    get_json(rx, Bucket::SocialCollections, id.as_bytes())?.ok_or_else(|| not_found("unknown collection"))
}

/// The share list only for the owner; works of hidden libraries for
/// nobody.
fn redact(viewer: &str, mut c: Collection, hidden: &Hidden) -> Collection {
    if viewer != c.owner_id {
        c.shared_with.clear();
    }
    c.items.retain(|it| !hidden_work(hidden, &it.work));
    c
}

fn clean_fields(name: Option<&mut String>, desc: Option<&mut String>, vis: Option<&String>) -> Result<(), Error> {
    if let Some(name) = name {
        let v = clean_text(name, crate::MAX_COLLECTION_NAME, "name", false)?;
        if v.is_empty() {
            return Err(invalid("name required"));
        }
        *name = v;
    }
    if let Some(desc) = desc {
        *desc = clean_text(desc, crate::MAX_COLLECTION_DESC, "description", true)?;
    }
    if let Some(v) = vis
        && !valid_visibility(v)
    {
        return Err(invalid("visibility must be public, friends or private"));
    }
    Ok(())
}

/// Share recipients: friends only, deduplicated, never the sender.
fn friend_targets(rx: &impl Rx, user_id: &str, to: &[String]) -> Result<Vec<String>, Error> {
    if to.is_empty() {
        return Err(invalid("choose at least one friend"));
    }
    if to.len() > crate::MAX_SHARE_TARGETS {
        return Err(invalid("too many recipients"));
    }
    let mut out: Vec<String> = Vec::new();
    for id in to {
        if id == user_id || out.contains(id) {
            continue;
        }
        if !are_friends(rx, user_id, id)? {
            return Err(forbidden("you can only share with friends"));
        }
        out.push(id.clone());
    }
    if out.is_empty() {
        return Err(invalid("choose at least one friend"));
    }
    Ok(out)
}

impl Social {
    pub(crate) fn invoke_collections(&self, op: &str, v: Value) -> Result<Value, Error> {
        use collections_op::*;
        let change = |v: Value| -> Result<ChangeCollectionInput, Error> {
            let i: ChangeCollectionInput = input(v, "a collection input")?;
            need_user(&[&i.user_id])?;
            Ok(i)
        };
        match op {
            LIST => {
                let i: ListCollectionsInput = input(v, "ListCollectionsInput")?;
                need_user(&[&i.viewer_id])?;
                out(self.collections(&i.viewer_id, &i.owner_id, &i.hidden_libraries.into_iter().collect())?)
            }
            GET => {
                let i: GetCollectionInput = input(v, "GetCollectionInput")?;
                need_user(&[&i.viewer_id])?;
                out(self.collection(&i.viewer_id, &i.id, &i.hidden_libraries.into_iter().collect())?)
            }
            CREATE => {
                let i: CreateCollectionInput = input(v, "CreateCollectionInput")?;
                need_user(&[&i.user_id])?;
                out(self.create_collection(i)?)
            }
            UPDATE => out(self.update_collection(change(v)?)?),
            DELETE => {
                let i = change(v)?;
                out(self.delete_collection(&i.user_id, &i.id)?)
            }
            ADD_ITEM => out(self.add_collection_item(change(v)?)?),
            REMOVE_ITEM => out(self.remove_collection_item(change(v)?)?),
            SHARE => out(self.share_collection(change(v)?)?),
            UNSHARE => {
                let i = change(v)?;
                need_user(&[&i.other_id])?;
                out(self.unshare_collection(i)?)
            }
            SHARE_WORK => {
                let i: ShareWorkInput = input(v, "ShareWorkInput")?;
                need_user(&[&i.user_id])?;
                out(self.share_work(i)?)
            }
            _ => Err(invalid(format!("unknown op {op} of lain.social.collections@1"))),
        }
    }

    /// Collections for a viewer (see [`ListCollectionsInput`]), newest
    /// change first.
    pub fn collections(&self, viewer: &str, owner: &str, hidden: &Hidden) -> Result<Vec<Collection>, Error> {
        let mut out = self.view(|r| {
            if !owner.is_empty() && owner != viewer && !can_see_tx(r, viewer, owner, SUBJECT_PROFILE)? {
                // The profile gate covers the list; each collection's own
                // visibility still applies on top.
                return Ok(Vec::new());
            }
            let mut out = Vec::new();
            for (_, c) in scan::<Collection>(r, Bucket::SocialCollections, b"")? {
                let keep = if owner.is_empty() && c.owner_id == viewer {
                    true
                } else if owner.is_empty() {
                    c.shared_with.iter().any(|id| id == viewer) && can_open(r, viewer, &c)?
                } else {
                    c.owner_id == owner && can_open(r, viewer, &c)?
                };
                if keep {
                    out.push(redact(viewer, c, hidden));
                }
            }
            Ok(out)
        })?;
        out.sort_by_key(|r| std::cmp::Reverse(r.updated_at));
        Ok(out)
    }

    /// Opens one collection; a hidden one reads as unknown.
    pub fn collection(&self, viewer: &str, id: &str, hidden: &Hidden) -> Result<Collection, Error> {
        self.view(|r| {
            let c = get_collection(r, id)?;
            if !can_open(r, viewer, &c)? {
                return Err(not_found("unknown collection"));
            }
            Ok(redact(viewer, c, hidden))
        })
    }

    /// Creates a collection, private unless told otherwise.
    pub fn create_collection(&self, mut i: CreateCollectionInput) -> Result<Collection, Error> {
        if i.visibility.is_empty() {
            i.visibility = visibility::PRIVATE.into();
        }
        clean_fields(Some(&mut i.name), Some(&mut i.description), Some(&i.visibility))?;
        let now = self.now();
        let c = Collection {
            id: self.new_id(),
            owner_id: i.user_id,
            name: i.name,
            description: i.description,
            visibility: i.visibility,
            shared_with: Vec::new(),
            items: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        self.update(|tx| tx.put(Bucket::SocialCollections, c.id.as_bytes(), &c))?;
        Ok(c)
    }

    /// Loads a collection the user owns, applies `f` and saves it.
    /// Someone else's collection reads as unknown when they cannot open
    /// it and as forbidden when they can.
    fn mutate_owned<T>(
        &self,
        user_id: &str,
        id: &str,
        f: impl FnOnce(&mut Tx<'_>, &mut Collection) -> Result<T, Error>,
    ) -> Result<(Collection, T), Error> {
        let now = self.now();
        self.update(|tx| {
            let mut c = get_collection(tx, id)?;
            if c.owner_id != user_id {
                if can_open(tx, user_id, &c)? {
                    return Err(forbidden("only the owner can change a collection"));
                }
                return Err(not_found("unknown collection"));
            }
            let t = f(tx, &mut c)?;
            c.updated_at = now;
            tx.put(Bucket::SocialCollections, c.id.as_bytes(), &c)?;
            Ok((c, t))
        })
    }

    /// Edits name, description or visibility.
    pub fn update_collection(&self, mut i: ChangeCollectionInput) -> Result<Collection, Error> {
        clean_fields(i.patch.name.as_mut(), i.patch.description.as_mut(), i.patch.visibility.as_ref())?;
        let hidden: Hidden = i.hidden_libraries.iter().cloned().collect();
        let p = i.patch.clone();
        let (c, ()) = self.mutate_owned(&i.user_id, &i.id, |_, c| {
            if let Some(v) = p.name {
                c.name = v;
            }
            if let Some(v) = p.description {
                c.description = v;
            }
            if let Some(v) = p.visibility {
                c.visibility = v;
            }
            Ok(())
        })?;
        Ok(redact(&i.user_id, c, &hidden))
    }

    /// Removes a collection the user owns.
    pub fn delete_collection(&self, user_id: &str, id: &str) -> Result<bool, Error> {
        self.update(|tx| {
            let c = get_collection(tx, id)?;
            if c.owner_id != user_id {
                return Err(not_found("unknown collection"));
            }
            tx.delete(Bucket::SocialCollections, id.as_bytes())?;
            Ok(true)
        })
    }

    /// Appends a work, or updates the note of one already there without
    /// moving it.
    pub fn add_collection_item(&self, i: ChangeCollectionInput) -> Result<Collection, Error> {
        let w = normalize_work(i.work.clone().unwrap_or_default())?;
        let note = clean_text(&i.note, crate::MAX_MESSAGE, "note", false)?;
        let hidden: Hidden = i.hidden_libraries.iter().cloned().collect();
        let now = self.now();
        let (c, ()) = self.mutate_owned(&i.user_id, &i.id, |_, c| {
            let key = work_key(&w);
            if let Some(it) = c.items.iter_mut().find(|it| work_key(&it.work) == key) {
                it.note = note;
                return Ok(());
            }
            if c.items.len() >= crate::MAX_COLLECTION_SIZE {
                return Err(invalid("a collection holds at most 500 works"));
            }
            c.items.push(CollectionItem { work: w, note, added_at: now });
            Ok(())
        })?;
        Ok(redact(&i.user_id, c, &hidden))
    }

    /// Removes a work; absent is not an error.
    pub fn remove_collection_item(&self, i: ChangeCollectionInput) -> Result<Collection, Error> {
        let w = normalize_work(i.work.clone().unwrap_or_default())?;
        let hidden: Hidden = i.hidden_libraries.iter().cloned().collect();
        let (c, ()) = self.mutate_owned(&i.user_id, &i.id, |_, c| {
            let key = work_key(&w);
            c.items.retain(|it| work_key(&it.work) != key);
            Ok(())
        })?;
        Ok(redact(&i.user_id, c, &hidden))
    }

    /// Grants friends read access and notifies them.
    pub fn share_collection(&self, i: ChangeCollectionInput) -> Result<ShareOutput, Error> {
        let msg = clean_text(&i.message, crate::MAX_MESSAGE, "message", false)?;
        let (_, sent) = self.mutate_owned(&i.user_id, &i.id, |tx, c| {
            let targets = friend_targets(tx, &i.user_id, &i.to)?;
            for id in &targets {
                if !c.shared_with.contains(id) {
                    c.shared_with.push(id.clone());
                }
                let (cid, m) = (c.id.clone(), msg.clone());
                self.notify_tx(tx, id, notify::COLLECTION_SHARED, &i.user_id, |n| {
                    n.collection_id = cid;
                    n.message = m;
                })?;
            }
            Ok(targets)
        })?;
        Ok(ShareOutput { sent })
    }

    /// Revokes one account's explicit access.
    pub fn unshare_collection(&self, i: ChangeCollectionInput) -> Result<Collection, Error> {
        let hidden: Hidden = i.hidden_libraries.iter().cloned().collect();
        let (c, ()) = self.mutate_owned(&i.user_id, &i.id, |_, c| {
            c.shared_with.retain(|id| *id != i.other_id);
            Ok(())
        })?;
        Ok(redact(&i.user_id, c, &hidden))
    }

    /// Recommends a work to friends through their notifications.
    pub fn share_work(&self, i: ShareWorkInput) -> Result<ShareOutput, Error> {
        let w = normalize_work(i.work)?;
        let msg = clean_text(&i.message, crate::MAX_MESSAGE, "message", false)?;
        self.update(|tx| {
            let targets = friend_targets(tx, &i.user_id, &i.to)?;
            for id in &targets {
                let (work, m) = (w.clone(), msg.clone());
                self.notify_tx(tx, id, notify::SHARE, &i.user_id, |n| {
                    n.work = Some(work);
                    n.message = m;
                })?;
            }
            Ok(ShareOutput { sent: targets })
        })
    }
}
