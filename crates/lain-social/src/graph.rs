//! `lain.social.graph@1`: privacy settings, favorites, relationships and
//! the one visibility rule.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use lain_api::{SocialSettingsPatch, notify, relation, visibility};
use lain_core::Error;
use lain_store::Bucket;

use crate::{
    Hidden, Notification, Relationship, Rx, Social, SocialSettings, Tx, forbidden, get_json, hidden_work, input,
    invalid, need_user, normalize_work, not_found, out, pair, prefix, scan, valid_visibility, work_key,
};

// Relationship actions.
pub const ACTION_REQUEST: &str = "request";
pub const ACTION_ACCEPT: &str = "accept";
pub const ACTION_DECLINE: &str = "decline";
pub const ACTION_REMOVE: &str = "remove";
pub const ACTION_BLOCK: &str = "block";
pub const ACTION_UNBLOCK: &str = "unblock";

// Visibility subjects.
pub const SUBJECT_PROFILE: &str = "profile";
pub const SUBJECT_ACTIVITY: &str = "activity";
pub const SUBJECT_RATINGS: &str = "ratings";

/// Ops of `lain.social.graph@1`.
pub mod graph_op {
    /// `{user_id, hidden_libraries?}` → `SocialSettings`
    pub const SETTINGS: &str = "settings";
    /// `{user_id, patch}` → `SocialSettings`
    pub const PATCH_SETTINGS: &str = "patch-settings";
    /// `{user_id, other_id}` → `{state, blocked_by}`
    pub const RELATION: &str = "relation";
    /// `{user_id, other_id, action}` → `{state, blocked_by}`
    pub const RELATE: &str = "relate";
    /// `{user_id}` → `[Relationship]`
    pub const RELATIONS: &str = "relations";
    /// `{viewer_id, owner_id, subject}` → bool
    pub const CAN_SEE: &str = "can-see";
}

pub const REQUESTS_EVERYONE: &str = "everyone";
pub const REQUESTS_NOBODY: &str = "nobody";

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SettingsInput {
    #[serde(default)]
    pub user_id: String,
    /// Favorites naming items of these libraries are left out (D-156).
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PatchSettingsInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub patch: SocialSettingsPatch,
    #[serde(default)]
    pub hidden_libraries: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RelationInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub other_id: String,
}

/// The viewer's relation. `blocked_by` reports a block placed by the
/// other side: the gateway answers "unknown user" and never forwards it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationOutput {
    pub state: String,
    #[serde(default)]
    pub blocked_by: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct RelateInput {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub other_id: String,
    #[serde(default)]
    pub action: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CanSeeInput {
    #[serde(default)]
    pub viewer_id: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub subject: String,
}

/// The stored side of a relationship: `state` is the owner's view;
/// `blocked_by` is kept apart so a mutual block survives either side
/// lifting theirs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Edge {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub state: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub blocked_by: bool,
    #[serde(default)]
    pub updated_at: i64,
}

/// What an account that never opened its privacy settings gets.
pub fn default_settings(user_id: &str) -> SocialSettings {
    SocialSettings {
        user_id: user_id.into(),
        profile: visibility::PUBLIC.into(),
        activity: visibility::FRIENDS.into(),
        ratings: visibility::FRIENDS.into(),
        discoverable: true,
        allow_requests: REQUESTS_EVERYONE.into(),
        hide_progress: false,
        favorites: Vec::new(),
        updated_at: 0,
    }
}

pub(crate) fn get_settings(rx: &impl Rx, user_id: &str) -> Result<SocialSettings, Error> {
    Ok(get_json(rx, Bucket::SocialSettings, user_id.as_bytes())?.unwrap_or_else(|| default_settings(user_id)))
}

pub(crate) fn get_edge(rx: &impl Rx, owner: &str, other: &str) -> Result<Edge, Error> {
    Ok(get_json(rx, Bucket::SocialEdges, &pair(owner, other))?.unwrap_or_default())
}

fn put_edge(tx: &mut Tx<'_>, owner: &str, other: &str, e: &Edge) -> Result<(), Error> {
    if e.state.is_empty() && !e.blocked_by {
        tx.delete(Bucket::SocialEdges, &pair(owner, other))?;
        return Ok(());
    }
    tx.put(Bucket::SocialEdges, &pair(owner, other), e)
}

fn relation_of(e: &Edge) -> RelationOutput {
    let state = if e.state.is_empty() { relation::NONE.to_string() } else { e.state.clone() };
    RelationOutput { state, blocked_by: e.blocked_by }
}

/// A block in either direction.
pub(crate) fn blocked_either(rx: &impl Rx, a: &str, b: &str) -> Result<bool, Error> {
    let e = get_edge(rx, a, b)?;
    Ok(e.state == relation::BLOCKING || e.blocked_by)
}

pub(crate) fn are_friends(rx: &impl Rx, a: &str, b: &str) -> Result<bool, Error> {
    Ok(get_edge(rx, a, b)?.state == relation::FRIEND)
}

pub(crate) fn friend_ids(rx: &impl Rx, user_id: &str) -> Result<Vec<String>, Error> {
    Ok(scan::<Edge>(rx, Bucket::SocialEdges, &prefix(user_id))?
        .into_iter()
        .filter(|(_, e)| e.state == relation::FRIEND)
        .map(|(id, _)| id)
        .collect())
}

/// The one privacy rule: owners see themselves, a block in either
/// direction hides everything, then the owner's level decides.
pub(crate) fn can_see_tx(rx: &impl Rx, viewer: &str, owner: &str, subject: &str) -> Result<bool, Error> {
    if viewer == owner {
        return Ok(true);
    }
    if blocked_either(rx, viewer, owner)? {
        return Ok(false);
    }
    let st = get_settings(rx, owner)?;
    let level = match subject {
        SUBJECT_PROFILE => st.profile,
        SUBJECT_ACTIVITY => st.activity,
        SUBJECT_RATINGS => st.ratings,
        _ => return Err(invalid("unknown visibility subject")),
    };
    level_allows(rx, &level, viewer, owner)
}

pub(crate) fn level_allows(rx: &impl Rx, level: &str, viewer: &str, owner: &str) -> Result<bool, Error> {
    match level {
        visibility::PUBLIC => Ok(true),
        visibility::FRIENDS => are_friends(rx, viewer, owner),
        _ => Ok(false),
    }
}

/// Favorites without works of hidden libraries.
fn visible_settings(mut st: SocialSettings, hidden: &Hidden) -> SocialSettings {
    st.favorites.retain(|w| !hidden_work(hidden, w));
    st
}

/// Marks recipient's friend-request notifications from sender read once
/// the request is answered or withdrawn.
fn settle_request_tx(tx: &mut Tx<'_>, recipient: &str, sender: &str) -> Result<(), Error> {
    let rows: Vec<(String, Notification)> = scan(tx, Bucket::SocialNotifications, &prefix(recipient))?;
    for (id, mut n) in rows {
        if !n.read && n.kind == notify::FRIEND_REQUEST && n.from_user_id == sender {
            n.read = true;
            tx.put(Bucket::SocialNotifications, &pair(recipient, &id), &n)?;
        }
    }
    Ok(())
}

impl Social {
    pub(crate) fn invoke_graph(&self, op: &str, v: Value) -> Result<Value, Error> {
        use graph_op::*;
        match op {
            SETTINGS => {
                let i: SettingsInput = input(v, "SettingsInput")?;
                need_user(&[&i.user_id])?;
                out(self.settings(&i.user_id, &i.hidden_libraries.into_iter().collect())?)
            }
            PATCH_SETTINGS => {
                let i: PatchSettingsInput = input(v, "PatchSettingsInput")?;
                need_user(&[&i.user_id])?;
                let st = self.patch_settings(&i.user_id, i.patch)?;
                out(visible_settings(st, &i.hidden_libraries.into_iter().collect()))
            }
            RELATION => {
                let i: RelationInput = input(v, "RelationInput")?;
                need_user(&[&i.user_id, &i.other_id])?;
                out(self.relation(&i.user_id, &i.other_id)?)
            }
            RELATE => {
                let i: RelateInput = input(v, "RelateInput")?;
                need_user(&[&i.user_id, &i.other_id])?;
                out(self.relate(&i.user_id, &i.other_id, &i.action)?)
            }
            RELATIONS => {
                let i: SettingsInput = input(v, "ListRelationsInput")?;
                need_user(&[&i.user_id])?;
                out(self.relations(&i.user_id)?)
            }
            CAN_SEE => {
                let i: CanSeeInput = input(v, "CanSeeInput")?;
                need_user(&[&i.viewer_id, &i.owner_id])?;
                out(self.can_see(&i.viewer_id, &i.owner_id, &i.subject)?)
            }
            _ => Err(invalid(format!("unknown op {op} of lain.social.graph@1"))),
        }
    }

    /// The account's settings, defaults when never saved.
    pub fn settings(&self, user_id: &str, hidden: &Hidden) -> Result<SocialSettings, Error> {
        self.view(|r| get_settings(r, user_id)).map(|st| visible_settings(st, hidden))
    }

    /// Validates and applies a settings patch.
    pub fn patch_settings(&self, user_id: &str, p: SocialSettingsPatch) -> Result<SocialSettings, Error> {
        for v in [&p.profile, &p.activity, &p.ratings].into_iter().flatten() {
            if !valid_visibility(v) {
                return Err(invalid("visibility must be public, friends or private"));
            }
        }
        if let Some(a) = &p.allow_requests
            && a != REQUESTS_EVERYONE
            && a != REQUESTS_NOBODY
        {
            return Err(invalid("allow_requests must be everyone or nobody"));
        }
        let favs = match &p.favorites {
            None => None,
            Some(list) => {
                if list.len() > crate::MAX_FAVORITES {
                    return Err(invalid("at most 12 favorites"));
                }
                let mut seen = std::collections::BTreeSet::new();
                let mut favs = Vec::new();
                for w in list {
                    let w = normalize_work(w.clone())?;
                    if seen.insert(work_key(&w)) {
                        favs.push(w);
                    }
                }
                Some(favs)
            }
        };
        let now = self.now();
        self.update(|tx| {
            let mut st = get_settings(tx, user_id)?;
            if let Some(v) = p.profile {
                st.profile = v;
            }
            if let Some(v) = p.activity {
                st.activity = v;
            }
            if let Some(v) = p.ratings {
                st.ratings = v;
            }
            if let Some(v) = p.discoverable {
                st.discoverable = v;
            }
            if let Some(v) = p.allow_requests {
                st.allow_requests = v;
            }
            if let Some(v) = p.hide_progress {
                st.hide_progress = v;
            }
            if let Some(f) = favs {
                st.favorites = f;
            }
            st.user_id = user_id.into();
            st.updated_at = now;
            tx.put(Bucket::SocialSettings, user_id.as_bytes(), &st)?;
            Ok(st)
        })
    }

    /// How `user_id` stands toward `other_id`.
    pub fn relation(&self, user_id: &str, other_id: &str) -> Result<RelationOutput, Error> {
        self.view(|r| get_edge(r, user_id, other_id)).map(|e| relation_of(&e))
    }

    /// Applies one relationship action and returns the new relation from
    /// the actor's side. Requests and acceptances notify the other
    /// account in the same transaction.
    pub fn relate(&self, user_id: &str, other_id: &str, action: &str) -> Result<RelationOutput, Error> {
        if user_id == other_id {
            return Err(invalid("cannot relate to yourself"));
        }
        let now = self.now();
        self.update(|tx| {
            let (mut mine, mut theirs) = (get_edge(tx, user_id, other_id)?, get_edge(tx, other_id, user_id)?);
            match action {
                ACTION_REQUEST | ACTION_ACCEPT => {
                    if mine.blocked_by {
                        // Never reveal the block: the account is simply not there.
                        return Err(not_found("unknown user"));
                    }
                    if mine.state == relation::BLOCKING {
                        return Err(forbidden("unblock this account first"));
                    }
                    match mine.state.as_str() {
                        relation::FRIEND | relation::OUTGOING => {}
                        relation::INCOMING => {
                            mine.state = relation::FRIEND.into();
                            theirs.state = relation::FRIEND.into();
                            settle_request_tx(tx, user_id, other_id)?;
                            self.notify_tx(tx, other_id, notify::FRIEND_ACCEPTED, user_id, |_| {})?;
                        }
                        _ => {
                            if action == ACTION_ACCEPT {
                                return Err(not_found("no pending request from this account"));
                            }
                            if get_settings(tx, other_id)?.allow_requests == REQUESTS_NOBODY {
                                return Err(forbidden("this account does not accept friend requests"));
                            }
                            mine.state = relation::OUTGOING.into();
                            theirs.state = relation::INCOMING.into();
                            self.notify_tx(tx, other_id, notify::FRIEND_REQUEST, user_id, |_| {})?;
                        }
                    }
                }
                ACTION_DECLINE => {
                    if mine.state != relation::INCOMING {
                        return Err(not_found("no pending request from this account"));
                    }
                    mine.state.clear();
                    theirs.state.clear();
                    settle_request_tx(tx, user_id, other_id)?;
                }
                ACTION_REMOVE => match mine.state.as_str() {
                    relation::OUTGOING => {
                        // A cancelled request must not stay actionable in their inbox.
                        settle_request_tx(tx, other_id, user_id)?;
                        mine.state.clear();
                        theirs.state.clear();
                    }
                    relation::FRIEND | relation::INCOMING => {
                        mine.state.clear();
                        theirs.state.clear();
                    }
                    _ => {}
                },
                ACTION_BLOCK => {
                    mine.state = relation::BLOCKING.into();
                    if theirs.state != relation::BLOCKING {
                        theirs.state.clear();
                    }
                    theirs.blocked_by = true;
                }
                ACTION_UNBLOCK => {
                    if mine.state == relation::BLOCKING {
                        mine.state.clear();
                        theirs.blocked_by = false;
                    }
                }
                _ => return Err(invalid("unknown relationship action")),
            }
            mine.updated_at = now;
            theirs.updated_at = now;
            put_edge(tx, user_id, other_id, &mine)?;
            put_edge(tx, other_id, user_id, &theirs)?;
            Ok(relation_of(&mine))
        })
    }

    /// The account's edges as it may see them: friends, pending requests
    /// both ways and its own blocks; blocks placed on it are omitted.
    pub fn relations(&self, user_id: &str) -> Result<Vec<Relationship>, Error> {
        self.view(|r| {
            Ok(scan::<Edge>(r, Bucket::SocialEdges, &prefix(user_id))?
                .into_iter()
                .filter(|(_, e)| !e.state.is_empty())
                .map(|(id, e)| Relationship { user_id: id, state: e.state, updated_at: e.updated_at })
                .collect())
        })
    }

    /// Whether `viewer` may see `owner`'s `subject`.
    pub fn can_see(&self, viewer: &str, owner: &str, subject: &str) -> Result<bool, Error> {
        self.view(|r| can_see_tx(r, viewer, owner, subject))
    }

    /// Stores one notification and trims the inbox; silent toward a
    /// recipient who blocked the sender (or was blocked), telling
    /// neither.
    pub(crate) fn notify_tx(
        &self,
        tx: &mut Tx<'_>,
        to: &str,
        kind: &str,
        from: &str,
        fill: impl FnOnce(&mut Notification),
    ) -> Result<(), Error> {
        if !from.is_empty() && blocked_either(tx, to, from)? {
            return Ok(());
        }
        let mut n = Notification {
            id: self.new_id(),
            user_id: to.into(),
            kind: kind.into(),
            from_user_id: from.into(),
            at: self.now(),
            ..Notification::default()
        };
        fill(&mut n);
        n.read = false;
        tx.put(Bucket::SocialNotifications, &pair(to, &n.id), &n)?;
        crate::activity::trim_prefix(tx, Bucket::SocialNotifications, &prefix(to), crate::MAX_NOTIFICATIONS)?;
        tx.notes.push(n);
        Ok(())
    }
}
