//! The social routes (`/api/social/*`). The gateway owns identity: who
//! is signed in, which accounts exist, which catalog item a work names
//! and which libraries this session may not see (D-156); the provider
//! behind `lain.social.*` owns every relationship and privacy rule.
//! Handlers resolve, call, and attach the public summaries of the
//! accounts an answer mentions. Go: `internal/gateway/social.go`.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use serde::Deserialize;
use serde_json::{Value, json};

use lain_api::{
    Activity, Collection, CollectionPatch, Notification, Rating, Relationship, SocialSettings, SocialSettingsPatch,
    UserSummary, WorkRef, relation,
};
use lain_auth::{ROLE_ADMIN, User, Verified};
use lain_core::{Error, codes};
use lain_social::{
    CAP_SOCIAL_ACTIVITY as ACTIVITY, CAP_SOCIAL_COLLECTIONS as COLLECTIONS, CAP_SOCIAL_GRAPH as GRAPH,
    CAP_SOCIAL_REVIEWS as REVIEWS, activity_op, collections_op, graph_op, reviews_op,
};

use crate::app::App;
use crate::catalog::CAP_CATALOG_READ;
use crate::contracts::CatalogItem;
use crate::gateway::{DELETE, GET, Gate, PATCH, POST, PUT, Reply, Req, route};
use crate::privacy;

pub fn routes() -> Router<Arc<App>> {
    use lain_social::{ACTION_BLOCK, ACTION_DECLINE, ACTION_REMOVE, ACTION_REQUEST, ACTION_UNBLOCK};
    Router::new()
        .route(
            "/api/social/me/settings",
            route(GET, Gate::User, settings_get).merge(route(PATCH, Gate::User, settings_patch)),
        )
        .route("/api/social/users", route(GET, Gate::User, user_search))
        .route("/api/social/users/{id}", route(GET, Gate::User, profile))
        .route("/api/social/users/{id}/activity", route(GET, Gate::User, user_activity))
        .route("/api/social/users/{id}/ratings", route(GET, Gate::User, user_ratings))
        .route("/api/social/users/{id}/collections", route(GET, Gate::User, user_collections))
        .route("/api/social/friends", route(GET, Gate::User, friends))
        .route(
            "/api/social/friends/{id}",
            route(POST, Gate::User, |a, r, v| relate(a, r, v, ACTION_REQUEST)).merge(route(
                DELETE,
                Gate::User,
                |a, r, v| relate(a, r, v, ACTION_REMOVE),
            )),
        )
        .route("/api/social/friends/{id}/decline", route(POST, Gate::User, |a, r, v| relate(a, r, v, ACTION_DECLINE)))
        .route(
            "/api/social/blocks/{id}",
            route(POST, Gate::User, |a, r, v| relate(a, r, v, ACTION_BLOCK)).merge(route(
                DELETE,
                Gate::User,
                |a, r, v| relate(a, r, v, ACTION_UNBLOCK),
            )),
        )
        .route("/api/social/feed", route(GET, Gate::User, feed))
        .route("/api/social/notifications", route(GET, Gate::User, notifications))
        .route("/api/social/notifications/read", route(POST, Gate::User, notifications_read))
        .route("/api/social/work", route(GET, Gate::User, work))
        .route(
            "/api/social/work/rating",
            route(PUT, Gate::User, rating_put).merge(route(DELETE, Gate::User, rating_delete)),
        )
        .route("/api/social/work/comments", route(POST, Gate::User, comment_add))
        .route("/api/social/comments/{id}", route(DELETE, Gate::User, comment_delete))
        .route("/api/social/share", route(POST, Gate::User, share))
        .route(
            "/api/social/collections",
            route(GET, Gate::User, collections).merge(route(POST, Gate::User, collection_create)),
        )
        .route(
            "/api/social/collections/{id}",
            route(GET, Gate::User, collection_get).merge(route(PATCH, Gate::User, collection_patch)).merge(route(
                DELETE,
                Gate::User,
                collection_delete,
            )),
        )
        .route(
            "/api/social/collections/{id}/items",
            route(POST, Gate::User, collection_add).merge(route(DELETE, Gate::User, collection_remove)),
        )
        .route("/api/social/collections/{id}/share", route(POST, Gate::User, collection_share))
        .route("/api/social/collections/{id}/share/{user}", route(DELETE, Gate::User, collection_unshare))
}

// ---- helpers ----------------------------------------------------------------

/// The public face of an account (what any signed-in account sees).
fn summary(u: &User) -> UserSummary {
    UserSummary {
        id: u.id.clone(),
        username: u.username.clone(),
        display_name: u.profile.display_name.clone(),
        avatar: u.profile.avatar.clone(),
    }
}

/// An existing, enabled account.
fn active_user(app: &App, id: &str) -> Option<User> {
    app.auth().get_public(id).filter(|u| !u.disabled)
}

/// The users map an answer carries; unknown and disabled accounts are
/// left out (clients show them as removed).
fn summaries<'a>(app: &App, ids: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, UserSummary> {
    let mut out = BTreeMap::new();
    for id in ids {
        if id.is_empty() || out.contains_key(id) {
            continue;
        }
        if let Some(u) = active_user(app, id) {
            out.insert(id.to_string(), summary(&u));
        }
    }
    out
}

/// Typed provider codes onto HTTP, the code kept in the body.
fn social_err(e: Error) -> Reply {
    let status = match e.code.as_str() {
        codes::INVALID_MESSAGE => 400,
        codes::NOT_FOUND => 404,
        lain_social::FORBIDDEN => 403,
        codes::INTERNAL => {
            tracing::warn!(err = %e.msg, "social call failed");
            return Reply::err(503, "social service unavailable");
        }
        _ => {
            tracing::warn!(code = %e.code, err = %e.msg, "social call failed");
            503
        }
    };
    Reply::json(status, json!({"error": e.msg, "code": e.code}))
}

fn call(app: &App, cap: &str, op: &str, input: Value) -> Result<Value, Reply> {
    app.registry.call_one(cap, op, input).map(|(v, _)| v).map_err(social_err)
}

fn typed<T: for<'de> Deserialize<'de> + Default>(v: Value) -> T {
    serde_json::from_value(v).unwrap_or_default()
}

/// The libraries this session may not see (D-156), for the provider.
fn hidden(app: &App, v: &Verified) -> Vec<String> {
    app.hidden_libraries(v)
}

fn unknown_user() -> Reply {
    Reply::json(404, json!({"error": "unknown user", "code": "not-found"}))
}

/// Fills a work from the catalog when the client names an item: the
/// catalog is authoritative for files it owns, and the item's library
/// travels with the work (D-156). An item this session may not see
/// reads exactly like one that does not exist: unknown without a kind
/// and title, else the work stands on those alone.
pub(crate) fn resolve_work(app: &App, v: &Verified, mut w: WorkRef) -> Result<WorkRef, Reply> {
    w.library_id.clear();
    if !w.item_id.is_empty() {
        match app.visible_item(v, &w.item_id) {
            Some(it) => {
                w.kind = it.kind;
                w.title = it.title;
                w.library_id = it.library_id;
            }
            None if w.kind.is_empty() || w.title.is_empty() => {
                return Err(social_err(Error::not_found("unknown item")));
            }
            None => w.item_id.clear(),
        }
    }
    lain_social::normalize_work(w).map_err(social_err)
}

fn work_from_query(req: &Req) -> WorkRef {
    WorkRef {
        item_id: req.query("item").into(),
        kind: req.query("kind").into(),
        title: req.query("title").into(),
        library_id: String::new(),
    }
}

#[derive(Deserialize, Default)]
struct RelationOut {
    #[serde(default)]
    state: String,
    #[serde(default)]
    blocked_by: bool,
}

/// The `{id}` account for an action. Unknown, disabled and blocked-by
/// accounts all read the same: not found.
fn target_user(app: &App, req: &Req, viewer: &str) -> Result<(User, String), Reply> {
    let id = req.param("id");
    let u = active_user(app, id).ok_or_else(unknown_user)?;
    if id == viewer {
        return Ok((u, relation::NONE.into()));
    }
    let rel: RelationOut = typed(call(app, GRAPH, graph_op::RELATION, json!({"user_id": viewer, "other_id": id}))?);
    if rel.blocked_by {
        return Err(unknown_user());
    }
    Ok((u, rel.state))
}

fn query_i64(req: &Req, key: &str) -> i64 {
    req.query(key).parse().unwrap_or(0)
}

macro_rules! tri {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(r) => return r,
        }
    };
}

// ---- settings ---------------------------------------------------------------

fn settings_get(app: &App, _: &Req, v: &Verified) -> Reply {
    let out =
        tri!(call(app, GRAPH, graph_op::SETTINGS, json!({"user_id": v.user_id, "hidden_libraries": hidden(app, v)})));
    Reply::ok(out)
}

fn settings_patch(app: &App, req: &Req, v: &Verified) -> Reply {
    let mut p: SocialSettingsPatch = tri!(req.decode());
    if let Some(favs) = p.favorites.take() {
        let mut out = Vec::with_capacity(favs.len());
        for f in favs {
            out.push(tri!(resolve_work(app, v, f)));
        }
        p.favorites = Some(out);
    }
    let input = json!({"user_id": v.user_id, "patch": p, "hidden_libraries": hidden(app, v)});
    Reply::ok(tri!(call(app, GRAPH, graph_op::PATCH_SETTINGS, input)))
}

// ---- people -----------------------------------------------------------------

/// Accounts by username or display name. Non-discoverable accounts
/// appear only to accounts already related to them; blocks either way
/// hide the account.
fn user_search(app: &App, req: &Req, v: &Verified) -> Reply {
    let q = req.query("q").trim().to_lowercase();
    let mut out = Vec::new();
    for u in app.auth().list().unwrap_or_default() {
        if u.id == v.user_id || u.disabled {
            continue;
        }
        if !q.is_empty()
            && !u.username.to_lowercase().contains(&q)
            && !u.profile.display_name.to_lowercase().contains(&q)
        {
            continue;
        }
        let rel: RelationOut =
            typed(tri!(call(app, GRAPH, graph_op::RELATION, json!({"user_id": v.user_id, "other_id": u.id}))));
        if rel.blocked_by || rel.state == relation::BLOCKING {
            continue;
        }
        if rel.state == relation::NONE {
            let st: SocialSettings = typed(tri!(call(app, GRAPH, graph_op::SETTINGS, json!({"user_id": u.id}))));
            if !st.discoverable {
                continue;
            }
        }
        out.push(json!({"user": summary(&u), "relation": rel.state}));
        if out.len() == 50 {
            break;
        }
    }
    Reply::ok(json!({"users": out}))
}

/// Another account's profile as the viewer may see it: identity always,
/// bio and favorites behind the profile level.
fn profile(app: &App, req: &Req, v: &Verified) -> Reply {
    let (u, rel) = tri!(target_user(app, req, &v.user_id));
    let mut vis = BTreeMap::new();
    for subject in [lain_social::SUBJECT_PROFILE, lain_social::SUBJECT_ACTIVITY, lain_social::SUBJECT_RATINGS] {
        let ok = tri!(call(
            app,
            GRAPH,
            graph_op::CAN_SEE,
            json!({"viewer_id": v.user_id, "owner_id": u.id, "subject": subject})
        ));
        vis.insert(subject, ok.as_bool().unwrap_or(false));
    }
    let mut resp = json!({
        "user": summary(&u),
        "relation": rel,
        "self": u.id == v.user_id,
        "visible": vis,
        "favorites": [],
    });
    if vis[lain_social::SUBJECT_PROFILE] {
        let st: SocialSettings = typed(tri!(call(
            app,
            GRAPH,
            graph_op::SETTINGS,
            json!({"user_id": u.id, "hidden_libraries": hidden(app, v)})
        )));
        resp["bio"] = json!(u.profile.bio);
        resp["favorites"] = json!(st.favorites);
    }
    Reply::ok(resp)
}

fn user_activity(app: &App, req: &Req, v: &Verified) -> Reply {
    let (u, _) = tri!(target_user(app, req, &v.user_id));
    let input = json!({
        "viewer_id": v.user_id, "owner_id": u.id, "before": query_i64(req, "before"),
        "limit": query_i64(req, "limit"), "hidden_libraries": hidden(app, v),
    });
    let items = tri!(call(app, ACTIVITY, activity_op::USER_ACTIVITY, input));
    Reply::ok(json!({"items": items, "users": summaries(app, [u.id.as_str()])}))
}

fn user_ratings(app: &App, req: &Req, v: &Verified) -> Reply {
    let (u, _) = tri!(target_user(app, req, &v.user_id));
    let input = json!({"viewer_id": v.user_id, "owner_id": u.id, "hidden_libraries": hidden(app, v)});
    Reply::ok(json!({"ratings": tri!(call(app, REVIEWS, reviews_op::USER_RATINGS, input))}))
}

fn user_collections(app: &App, req: &Req, v: &Verified) -> Reply {
    let (u, _) = tri!(target_user(app, req, &v.user_id));
    let input = json!({"viewer_id": v.user_id, "owner_id": u.id, "hidden_libraries": hidden(app, v)});
    let cols = tri!(call(app, COLLECTIONS, collections_op::LIST, input));
    Reply::ok(json!({"collections": cols, "users": summaries(app, [u.id.as_str()])}))
}

fn friends(app: &App, _: &Req, v: &Verified) -> Reply {
    let rels: Vec<Relationship> = typed(tri!(call(app, GRAPH, graph_op::RELATIONS, json!({"user_id": v.user_id}))));
    let mut groups: BTreeMap<&str, Vec<Value>> =
        [relation::FRIEND, relation::INCOMING, relation::OUTGOING, relation::BLOCKING]
            .into_iter()
            .map(|k| (k, vec![]))
            .collect();
    for r in rels {
        let Some(u) = app.auth().get_public(&r.user_id) else { continue };
        if u.disabled && r.state != relation::BLOCKING {
            continue;
        }
        if let Some(g) = groups.get_mut(r.state.as_str()) {
            g.push(json!({"user": summary(&u), "since": r.updated_at}));
        }
    }
    Reply::ok(json!({
        "friends": groups[relation::FRIEND],
        "incoming": groups[relation::INCOMING],
        "outgoing": groups[relation::OUTGOING],
        "blocked": groups[relation::BLOCKING],
    }))
}

fn relate(app: &App, req: &Req, v: &Verified, action: &str) -> Reply {
    // Unblocking must still reach an account that blocked you back, so
    // only the existence check runs before the provider decides.
    let Some(u) = app.auth().get_public(req.param("id")) else { return unknown_user() };
    if u.disabled && action != lain_social::ACTION_UNBLOCK && action != lain_social::ACTION_REMOVE {
        return unknown_user();
    }
    let input = json!({"user_id": v.user_id, "other_id": u.id, "action": action});
    let rel: RelationOut = typed(tri!(call(app, GRAPH, graph_op::RELATE, input)));
    Reply::ok(json!({"user": summary(&u), "relation": rel.state}))
}

// ---- feed and notifications -------------------------------------------------

fn feed(app: &App, req: &Req, v: &Verified) -> Reply {
    let input = json!({
        "user_id": v.user_id, "before": query_i64(req, "before"), "limit": query_i64(req, "limit"),
        "hidden_libraries": hidden(app, v),
    });
    let rows: Vec<Activity> = typed(tri!(call(app, ACTIVITY, activity_op::FEED, input)));
    let users = summaries(app, rows.iter().map(|a| a.user_id.as_str()));
    let items: Vec<&Activity> = rows.iter().filter(|a| users.contains_key(&a.user_id)).collect();
    Reply::ok(json!({"items": items, "users": users}))
}

fn notifications(app: &App, req: &Req, v: &Verified) -> Reply {
    #[derive(Deserialize, Default)]
    struct Out {
        #[serde(default)]
        items: Vec<Notification>,
        #[serde(default)]
        unread: i64,
    }
    let input = json!({"user_id": v.user_id, "limit": query_i64(req, "limit"), "hidden_libraries": hidden(app, v)});
    let n: Out = typed(tri!(call(app, ACTIVITY, activity_op::NOTIFICATIONS, input)));
    let users = summaries(app, n.items.iter().map(|i| i.from_user_id.as_str()));
    Reply::ok(json!({"items": n.items, "unread": n.unread, "users": users}))
}

fn notifications_read(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: lain_api::MarkRead = tri!(req.decode());
    let changed =
        tri!(call(app, ACTIVITY, activity_op::MARK_READ, json!({"user_id": v.user_id, "ids": i.ids, "all": i.all})));
    Reply::ok(json!({"changed": changed}))
}

/// Reports an accepted progress write to the activity log. Best effort:
/// social bookkeeping never fails a progress write, but a failure is
/// logged (a disabled social module is not a failure).
pub(crate) fn record_progress(app: &App, user_id: &str, it: &CatalogItem, completed: bool, was_completed: bool) {
    let input = json!({
        "user_id": user_id,
        "work": {"kind": it.kind, "title": it.title, "item_id": it.id, "library_id": it.library_id},
        "season": it.season, "episode": it.episode,
        "completed": completed, "was_completed": was_completed,
    });
    if let Err(e) = app.registry.call_one(ACTIVITY, activity_op::RECORD_PROGRESS, input)
        && e.code != codes::MODULE_DISABLED
    {
        tracing::warn!(user = user_id, item = %it.id, err = %e, "social activity not recorded");
    }
}

/// `social-notification` to the recipient's streams: the id, type and
/// sender only (clients re-read the inbox), and nothing at all for a
/// notification naming an item of a private library to a session that
/// has not unlocked it (D-156).
pub(crate) fn publish_notification(app: &App, n: &Notification) {
    let lib = n
        .work
        .as_ref()
        .filter(|w| !w.library_id.is_empty())
        .and_then(|w| app.libraries.get(&w.library_id))
        .filter(|l| l.private);
    let data = json!({"id": n.id, "type": n.kind, "from_user_id": n.from_user_id});
    let to = n.user_id.clone();
    app.events.publish_where("social-notification", data, &|s| {
        s.user_id == to && lib.as_ref().is_none_or(|l| privacy::items_visible(l, &app.unlocks, &s.user_id, &s.session))
    });
}

// ---- works: ratings, reviews, comments, shares ------------------------------

fn work(app: &App, req: &Req, v: &Verified) -> Reply {
    let w = tri!(resolve_work(app, v, work_from_query(req)));
    let view = tri!(call(
        app,
        REVIEWS,
        reviews_op::WORK,
        json!({"viewer_id": v.user_id, "work": w, "hidden_libraries": hidden(app, v)})
    ));
    let ratings: Vec<Rating> = typed(view["ratings"].clone());
    let comments: Vec<lain_api::Comment> = typed(view["comments"].clone());
    let ids = std::iter::once(v.user_id.as_str())
        .chain(ratings.iter().map(|r| r.user_id.as_str()))
        .chain(comments.iter().map(|c| c.user_id.as_str()));
    let users = summaries(app, ids);
    Reply::ok(json!({
        "work": view["work"], "mine": view.get("mine").cloned().unwrap_or(Value::Null), "ratings": view["ratings"],
        "average": view["average"], "scored": view["scored"], "comments": view["comments"], "users": users,
    }))
}

fn rating_put(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: lain_api::RatingInput = tri!(req.decode());
    let w = tri!(resolve_work(app, v, i.work));
    let input = json!({"user_id": v.user_id, "work": w, "score": i.score, "review": i.review, "spoiler": i.spoiler});
    Reply::ok(tri!(call(app, REVIEWS, reviews_op::PUT_RATING, input)))
}

fn rating_delete(app: &App, req: &Req, v: &Verified) -> Reply {
    let w = tri!(resolve_work(app, v, work_from_query(req)));
    let removed = tri!(call(app, REVIEWS, reviews_op::DELETE_RATING, json!({"user_id": v.user_id, "work": w})));
    Reply::ok(json!({"removed": removed}))
}

fn comment_add(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: lain_api::CommentInput = tri!(req.decode());
    let w = tri!(resolve_work(app, v, i.work));
    let input = json!({"user_id": v.user_id, "work": w, "body": i.body, "reply_to": i.reply_to});
    Reply::json(201, tri!(call(app, REVIEWS, reviews_op::ADD_COMMENT, input)))
}

fn comment_delete(app: &App, req: &Req, v: &Verified) -> Reply {
    let input = json!({"user_id": v.user_id, "id": req.param("id"), "moderator": v.role == ROLE_ADMIN});
    tri!(call(app, REVIEWS, reviews_op::DELETE_COMMENT, input));
    Reply::ok(json!({"removed": true}))
}

fn share(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: lain_api::ShareInput = tri!(req.decode());
    let w = tri!(resolve_work(app, v, i.work));
    let input = json!({"user_id": v.user_id, "to": i.to, "work": w, "message": i.message});
    Reply::ok(tri!(call(app, COLLECTIONS, collections_op::SHARE_WORK, input)))
}

// ---- collections ------------------------------------------------------------

fn collection_reply(app: &App, status: u16, out: Result<Value, Reply>) -> Reply {
    let out = tri!(out);
    let c: Collection = typed(out.clone());
    let ids = std::iter::once(c.owner_id.as_str()).chain(c.shared_with.iter().map(String::as_str));
    Reply::json(status, json!({"collection": out, "users": summaries(app, ids)}))
}

fn collections(app: &App, _: &Req, v: &Verified) -> Reply {
    let cols = tri!(call(
        app,
        COLLECTIONS,
        collections_op::LIST,
        json!({"viewer_id": v.user_id, "hidden_libraries": hidden(app, v)})
    ));
    let list: Vec<Collection> = typed(cols.clone());
    Reply::ok(json!({"collections": cols, "users": summaries(app, list.iter().map(|c| c.owner_id.as_str()))}))
}

fn collection_create(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: lain_api::NewCollection = tri!(req.decode());
    let input = json!({"user_id": v.user_id, "name": i.name, "description": i.description, "visibility": i.visibility});
    collection_reply(app, 201, call(app, COLLECTIONS, collections_op::CREATE, input))
}

fn collection_get(app: &App, req: &Req, v: &Verified) -> Reply {
    let input = json!({"viewer_id": v.user_id, "id": req.param("id"), "hidden_libraries": hidden(app, v)});
    collection_reply(app, 200, call(app, COLLECTIONS, collections_op::GET, input))
}

fn change(app: &App, req: &Req, v: &Verified, extra: Value) -> Value {
    let mut input = json!({"user_id": v.user_id, "id": req.param("id"), "hidden_libraries": hidden(app, v)});
    if let (Value::Object(m), Value::Object(e)) = (&mut input, extra) {
        m.extend(e);
    }
    input
}

fn collection_patch(app: &App, req: &Req, v: &Verified) -> Reply {
    let p: CollectionPatch = tri!(req.decode());
    collection_reply(app, 200, call(app, COLLECTIONS, collections_op::UPDATE, change(app, req, v, json!(p))))
}

fn collection_delete(app: &App, req: &Req, v: &Verified) -> Reply {
    tri!(call(app, COLLECTIONS, collections_op::DELETE, change(app, req, v, json!({}))));
    Reply::ok(json!({"removed": true}))
}

fn collection_add(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: lain_api::CollectionItemInput = tri!(req.decode());
    let w = tri!(resolve_work(app, v, i.work));
    let input = change(app, req, v, json!({"work": w, "note": i.note}));
    collection_reply(app, 200, call(app, COLLECTIONS, collections_op::ADD_ITEM, input))
}

fn collection_remove(app: &App, req: &Req, v: &Verified) -> Reply {
    let w = tri!(resolve_work(app, v, work_from_query(req)));
    let input = change(app, req, v, json!({"work": w}));
    collection_reply(app, 200, call(app, COLLECTIONS, collections_op::REMOVE_ITEM, input))
}

fn collection_share(app: &App, req: &Req, v: &Verified) -> Reply {
    let i: lain_api::CollectionShareInput = tri!(req.decode());
    let input = change(app, req, v, json!({"to": i.to, "message": i.message}));
    Reply::ok(tri!(call(app, COLLECTIONS, collections_op::SHARE, input)))
}

fn collection_unshare(app: &App, req: &Req, v: &Verified) -> Reply {
    let input = change(app, req, v, json!({"other_id": req.param("user")}));
    collection_reply(app, 200, call(app, COLLECTIONS, collections_op::UNSHARE, input))
}

/// A catalog item without the visibility check (progress writes are
/// recorded for the account whatever this session sees; reads filter).
pub(crate) fn catalog_item(app: &App, id: &str) -> Option<CatalogItem> {
    let (out, _) = app.registry.call_one(CAP_CATALOG_READ, "get", json!({"id": id})).ok()?;
    serde_json::from_value(out).ok()
}
