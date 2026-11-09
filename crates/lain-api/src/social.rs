//! The social module's wire (docs/slices/social.md): privacy settings,
//! relationships, activity, notifications, ratings, comments and
//! collections. Go: `internal/contracts/social.go` and the response
//! maps of `internal/gateway/social.go`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Avatar, is_false, is_zero_i64};

/// Visibility levels; public means every signed-in account here.
pub mod visibility {
    pub const PUBLIC: &str = "public";
    pub const FRIENDS: &str = "friends";
    pub const PRIVATE: &str = "private";
}

/// How the viewer stands toward another account. A block placed by the
/// other side is never reported.
pub mod relation {
    pub const NONE: &str = "none";
    pub const FRIEND: &str = "friend";
    pub const OUTGOING: &str = "outgoing";
    pub const INCOMING: &str = "incoming";
    pub const BLOCKING: &str = "blocking";
}

/// Activity row types.
pub mod activity_type {
    pub const PROGRESS: &str = "progress";
    pub const COMPLETED: &str = "completed";
    pub const RATED: &str = "rated";
}

/// Notification types.
pub mod notify {
    pub const FRIEND_REQUEST: &str = "friend_request";
    pub const FRIEND_ACCEPTED: &str = "friend_accepted";
    pub const SHARE: &str = "share";
    pub const COLLECTION_SHARED: &str = "collection_shared";
    pub const REPLY: &str = "reply";
}

/// A work of any media kind: identity is `kind` plus the normalized
/// title. `item_id` links back to a catalog item; `library_id` is that
/// item's library, set only by the server (D-156: rows naming an item
/// of a library this session may not see are left out).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRef {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub item_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub library_id: String,
}

/// One account's privacy configuration plus favorites.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialSettings {
    #[serde(default)]
    pub user_id: String,
    /// Gates bio, favorites and the collections list.
    #[serde(default)]
    pub profile: String,
    /// Gates activity rows in feeds and on the profile.
    #[serde(default)]
    pub activity: String,
    /// Gates ratings and reviews.
    #[serde(default)]
    pub ratings: String,
    /// Lists the account in user search.
    #[serde(default)]
    pub discoverable: bool,
    /// `everyone` or `nobody`.
    #[serde(default)]
    pub allow_requests: String,
    /// Keeps "watching/reading now" rows out of the feed.
    #[serde(default)]
    pub hide_progress: bool,
    #[serde(default)]
    pub favorites: Vec<WorkRef>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub updated_at: i64,
}

/// `PATCH /api/social/me/settings`: only the fields present apply.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialSettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ratings: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discoverable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_requests: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_progress: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorites: Option<Vec<WorkRef>>,
}

/// One relationship edge as its owner sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relationship {
    pub user_id: String,
    pub state: String,
    #[serde(default)]
    pub updated_at: i64,
}

/// One row of an account's activity log.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub season: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub episode: i64,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub score: i64,
    #[serde(default)]
    pub at: i64,
}

/// One event addressed to an account.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(default)]
    pub from_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work: Option<WorkRef>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub collection_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub at: i64,
}

/// One account's score (1..10) and optional review of a work.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rating {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub score: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub review: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub spoiler: bool,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// A remark on a work, optionally replying to another.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default)]
    pub body: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reply_to: String,
    #[serde(default)]
    pub created_at: i64,
}

/// One work in a collection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionItem {
    #[serde(default)]
    pub work: WorkRef,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    #[serde(default)]
    pub added_at: i64,
}

/// A named, ordered, shareable set of works.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collection {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub visibility: String,
    #[serde(default)]
    pub shared_with: Vec<String>,
    #[serde(default)]
    pub items: Vec<CollectionItem>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

/// The public face of an account in social answers.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub display_name: String,
    #[serde(default)]
    pub avatar: Avatar,
}

/// The accounts an answer mentions, by id (unknown and disabled
/// accounts are left out; clients show them as removed).
pub type UserMap = BTreeMap<String, UserSummary>;

/// One row of `GET /api/social/users`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserWithRelation {
    pub user: UserSummary,
    pub relation: String,
}

/// `GET /api/social/users`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSearch {
    pub users: Vec<UserWithRelation>,
}

/// What a profile shows the viewer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileVisibility {
    pub profile: bool,
    pub activity: bool,
    pub ratings: bool,
}

/// `GET /api/social/users/{id}`: identity always, bio and favorites
/// behind the profile level.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialProfile {
    pub user: UserSummary,
    pub relation: String,
    #[serde(rename = "self")]
    pub is_self: bool,
    pub visible: ProfileVisibility,
    pub favorites: Vec<WorkRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
}

/// `GET /api/social/feed` and `/users/{id}/activity`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityPage {
    pub items: Vec<Activity>,
    #[serde(default)]
    pub users: UserMap,
}

/// `GET /api/social/users/{id}/ratings`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ratings {
    pub ratings: Vec<Rating>,
}

/// One row of `GET /api/social/friends`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationRow {
    pub user: UserSummary,
    #[serde(default)]
    pub since: i64,
}

/// `GET /api/social/friends`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Friends {
    pub friends: Vec<RelationRow>,
    pub incoming: Vec<RelationRow>,
    pub outgoing: Vec<RelationRow>,
    pub blocked: Vec<RelationRow>,
}

/// The answer of every friend and block action.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelateReply {
    pub user: UserSummary,
    pub relation: String,
}

/// `GET /api/social/notifications`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notifications {
    pub items: Vec<Notification>,
    #[serde(default)]
    pub unread: i64,
    #[serde(default)]
    pub users: UserMap,
}

/// `POST /api/social/notifications/read`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkRead {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ids: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub all: bool,
}

/// The answer of `POST /api/social/notifications/read`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changed {
    pub changed: i64,
}

/// `GET /api/social/work`: the viewer's own rating, the other ratings
/// the viewer may see with their average, and the comments.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkView {
    pub work: WorkRef,
    /// `null` when the viewer has not rated the work.
    #[serde(default)]
    pub mine: Option<Rating>,
    pub ratings: Vec<Rating>,
    #[serde(default)]
    pub average: f64,
    #[serde(default)]
    pub scored: i64,
    pub comments: Vec<Comment>,
    #[serde(default)]
    pub users: UserMap,
}

/// `PUT /api/social/work/rating`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatingInput {
    pub work: WorkRef,
    #[serde(default)]
    pub score: i64,
    #[serde(default)]
    pub review: String,
    #[serde(default)]
    pub spoiler: bool,
}

/// `POST /api/social/work/comments`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentInput {
    pub work: WorkRef,
    pub body: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reply_to: String,
}

/// `POST /api/social/share`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareInput {
    pub to: Vec<String>,
    pub work: WorkRef,
    #[serde(default)]
    pub message: String,
}

/// Who received a share.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareReply {
    pub sent: Vec<String>,
}

/// `{"removed": bool}` (rating, comment and collection deletes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removed {
    pub removed: bool,
}

/// `GET /api/social/collections` and `/users/{id}/collections`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collections {
    pub collections: Vec<Collection>,
    #[serde(default)]
    pub users: UserMap,
}

/// One collection with its owner and (for the owner) share targets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionReply {
    pub collection: Collection,
    #[serde(default)]
    pub users: UserMap,
}

/// `POST /api/social/collections`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewCollection {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// `private` unless told otherwise.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub visibility: String,
}

/// `PATCH /api/social/collections/{id}`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,
}

/// `POST /api/social/collections/{id}/items`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionItemInput {
    pub work: WorkRef,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// `POST /api/social/collections/{id}/share`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionShareInput {
    pub to: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message: String,
}

/// `social-notification` on `/api/events`, to the recipient's streams:
/// a notification was stored. It carries no work or message (clients
/// re-read `/api/social/notifications`), and one naming an item of a
/// private library reaches only sessions that unlocked it (D-156).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SocialNotificationEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub from_user_id: String,
}
