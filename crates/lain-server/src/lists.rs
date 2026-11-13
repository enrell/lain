//! The lists routes (D-078..D-083): `GET /api/list`, the linked
//! accounts under `/api/me/links`, the OAuth landing
//! `GET /api/auth/{platform}/callback`, and the operator's integration
//! settings; a title's tracking and edits written back (D-288, D-289).
//! Thin: the work lives in `lain_lists::Links`, through the registry;
//! what is here is the join of list entries with library titles, which
//! needs the catalog and the metadata overlays. Go:
//! `internal/gateway/list.go` and `scrobble.go`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use lain_admin::integrations;
use lain_api::{Enrichment, ListEntry, ListEntryPatch, TitleTrack, list_match, list_media};
use lain_auth::Verified;
use lain_core::Error;
use lain_lists::links::{code_from_paste, err_text};
use lain_lists::scrobble::{Finished, is_reading};
use lain_lists::{code, match_key};
use lain_metadata::{ANILIST_ID, CAP_METADATA_ENRICH};

use crate::app::App;
use crate::catalog::{CAP_CATALOG_READ, title_key, watch_order};
use crate::contracts::CatalogItem;
use crate::gateway::{DELETE, GET, Gate, PATCH, POST, PUT, Reply, Req, base_url, route};

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/api/list", route(GET, Gate::User, list))
        .route("/api/list/{platform}/{remote_id}", route(PUT, Gate::User, entry_put))
        .route("/api/items/{id}/tracking", route(GET, Gate::User, tracking))
        .route("/api/me/links", route(GET, Gate::User, links))
        .route(
            "/api/me/links/{platform}",
            route(PATCH, Gate::User, link_patch).merge(route(DELETE, Gate::User, unlink)),
        )
        .route("/api/me/links/{platform}/authorize", route(GET, Gate::User, authorize))
        .route("/api/me/links/{platform}/pin", route(GET, Gate::User, pin))
        .route("/api/me/links/{platform}/code", route(POST, Gate::User, pasted_code))
        .route("/api/me/links/{platform}/sync", route(POST, Gate::User, sync))
        .route("/api/auth/{platform}/callback", route(GET, Gate::Public, callback))
        .route(
            "/api/admin/settings/integrations",
            route(GET, Gate::Admin, integrations_get).merge(route(PUT, Gate::Admin, integrations_put)),
        )
}

fn now() -> i64 {
    lain_core::now_unix()
}

/// Go's `writeLinkErr`: the code decides the status, and stays in the
/// body.
fn link_err(e: &Error) -> Reply {
    let status = match e.code.as_str() {
        code::UNSUPPORTED_PLATFORM => 404,
        code::TOKEN_INVALID | code::TOKEN_EXPIRED => 401,
        "invalid-message" | code::INVALID_GRANT => 400,
        "not-found" => 404,
        code::RATE_LIMITED => 429,
        _ => 503,
    };
    Reply::json(status, json!({"error": err_text(e), "code": e.code}))
}

fn not_configured(what: &str, platform: &str) -> Reply {
    Reply::json(503, json!({"error": format!("no {what} configured for {platform}"), "code": code::NOT_CONFIGURED}))
}

fn not_linked(platform: &str) -> Reply {
    Reply::err(404, &format!("no {platform} account linked"))
}

/// The redirect URI the platform sends the user back to, derived from
/// the request the client used (the admin page prints the same value).
fn callback_url(req: &Req, platform: &str) -> String {
    format!("{}/api/auth/{platform}/callback", base_url(req))
}

fn list(app: &App, req: &Req, v: &Verified) -> Reply {
    match app.links.list(&v.user_id, req.query("type"), req.query("status")) {
        Ok(mut entries) => {
            if !entries.is_empty() {
                Shelf::read(app, v).mark(&mut entries);
            }
            Reply::ok(json!({"entries": entries}))
        }
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

// ---- list entries x library titles (D-288) ---------------------------------

/// One library title as the list sees it: the file its page opens on
/// (the first in watch order), whether it is read, the names it goes
/// by and the platform ids its metadata names.
#[derive(Debug, Clone, Default)]
struct Title {
    item_id: String,
    reading: bool,
    keys: Vec<String>,
    /// Platform → ids (AniList's metadata provider names the AniList id).
    remote_ids: BTreeMap<String, Vec<String>>,
}

/// The titles of the libraries a session may see, minus private ones:
/// a private library's titles are never tied to an outside list
/// (D-156).
#[derive(Debug, Default)]
struct Shelf {
    titles: Vec<Title>,
}

/// The metadata provider whose ids are a platform's own.
fn platform_of(provider: &str) -> Option<&'static str> {
    (provider == ANILIST_ID).then_some(lain_lists::anilist::PLATFORM)
}

impl Shelf {
    fn read(app: &App, v: &Verified) -> Shelf {
        let hidden = app.hidden_libraries(v);
        let private: Vec<String> =
            app.libraries.list().unwrap_or_default().into_iter().filter(|l| l.private).map(|l| l.id).collect();
        let items: Vec<CatalogItem> = app
            .registry
            .call_one(CAP_CATALOG_READ, "list", Value::Null)
            .ok()
            .and_then(|(v, _)| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let items: Vec<CatalogItem> =
            items.into_iter().filter(|i| !hidden.contains(&i.library_id) && !private.contains(&i.library_id)).collect();
        let overlays = enrichments(app, &items_ids(&items));
        Shelf::build(items, &overlays)
    }

    /// Groups files into titles as the title page does (the catalog's
    /// `title_key`), and gathers each title's names and ids.
    fn build(mut items: Vec<CatalogItem>, overlays: &[Enrichment]) -> Shelf {
        items.sort_by(|a, b| title_key(&a.title).cmp(&title_key(&b.title)).then_with(|| watch_order(a, b)));
        let by_item: HashMap<&str, &Enrichment> = overlays.iter().map(|e| (e.item_id.as_str(), e)).collect();
        let mut titles: Vec<Title> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        for it in &items {
            let group = title_key(&it.title);
            let i = *at.entry(group).or_insert_with(|| {
                titles.push(Title { item_id: it.id.clone(), reading: is_reading(&it.kind), ..Title::default() });
                titles.len() - 1
            });
            let t = &mut titles[i];
            let overlay = by_item.get(it.id.as_str());
            let names = std::iter::once(&it.title).chain(it.aliases.iter()).chain(overlay.map(|o| &o.title));
            for k in names.map(|n| match_key(n)).filter(|k| !k.is_empty()) {
                if !t.keys.contains(&k) {
                    t.keys.push(k);
                }
            }
            if let Some(o) = overlay
                && let Some(p) = platform_of(&o.provider)
                && !o.remote_id.is_empty()
            {
                let ids = t.remote_ids.entry(p.to_string()).or_default();
                if !ids.contains(&o.remote_id) {
                    ids.push(o.remote_id.clone());
                }
            }
        }
        Shelf { titles }
    }

    /// The title an entry is: the one whose metadata names its id, else
    /// the one title that goes by one of its names. Two titles by the
    /// same name are never guessed between.
    fn find(&self, e: &ListEntry) -> Option<(&Title, &'static str)> {
        let reading = matches!(e.media_type.as_str(), list_media::MANGA | list_media::COMIC);
        let kind = self.titles.iter().filter(|t| t.reading == reading);
        if let Some(t) =
            kind.clone().find(|t| t.remote_ids.get(&e.platform).is_some_and(|ids| ids.contains(&e.remote_id)))
        {
            return Some((t, list_match::ID));
        }
        let keys: Vec<String> = std::iter::once(&e.title)
            .chain(e.alt_titles.iter())
            .map(|n| match_key(n))
            .filter(|k| !k.is_empty())
            .collect();
        let mut hits = kind.filter(|t| t.keys.iter().any(|k| keys.contains(k)));
        let t = hits.next()?;
        if hits.next().is_some() {
            return None;
        }
        Some((t, list_match::TITLE))
    }

    fn mark(&self, entries: &mut [ListEntry]) {
        for e in entries {
            if let Some((t, how)) = self.find(e) {
                e.item_id = t.item_id.clone();
                e.matched_by = how.into();
            }
        }
    }

    /// The title a file belongs to.
    fn title_of(&self, item_id: &str, all: &[CatalogItem]) -> Option<&Title> {
        let it = all.iter().find(|i| i.id == item_id)?;
        let group = title_key(&it.title);
        let first = all.iter().filter(|i| title_key(&i.title) == group).min_by(|a, b| watch_order(a, b))?;
        self.titles.iter().find(|t| t.item_id == first.id)
    }
}

fn items_ids(items: &[CatalogItem]) -> Vec<String> {
    items.iter().map(|i| i.id.clone()).collect()
}

fn enrichments(app: &App, ids: &[String]) -> Vec<Enrichment> {
    if ids.is_empty() {
        return Vec::new();
    }
    app.registry
        .call_one(CAP_METADATA_ENRICH, "batch", json!({"ids": ids}))
        .ok()
        .and_then(|(v, _)| serde_json::from_value(v["items"].clone()).ok())
        .unwrap_or_default()
}

/// `GET /api/items/{id}/tracking`: each linked platform's entry for the
/// title the item belongs to, from the stored copy (never a remote
/// call, so the page never waits on the platform).
fn tracking(app: &App, req: &Req, v: &Verified) -> Reply {
    let Some(item) = app.visible_item(v, req.param("id")) else { return Reply::err(404, "unknown item") };
    let accounts = match app.links.accounts(&v.user_id) {
        Ok(a) => a,
        Err(e) => return Reply::err(503, &e.to_string()),
    };
    // A private library's titles are not tied to an outside list.
    if accounts.is_empty() || app.libraries.get(&item.library_id).is_some_and(|l| l.private) {
        return Reply::ok(json!({"tracks": []}));
    }
    let group: Vec<CatalogItem> = app
        .registry
        .call_one(CAP_CATALOG_READ, "episodes", json!({"id": item.id}))
        .ok()
        .and_then(|(v, _)| serde_json::from_value(v).ok())
        .unwrap_or_else(|| vec![item.clone()]);
    let hidden = app.hidden_libraries(v);
    let group: Vec<CatalogItem> = group.into_iter().filter(|i| !hidden.contains(&i.library_id)).collect();
    let shelf = Shelf::build(group.clone(), &enrichments(app, &items_ids(&group)));
    let Some(title) = shelf.title_of(&item.id, &group) else { return Reply::ok(json!({"tracks": []})) };
    let mut entries = app.links.list(&v.user_id, "", "").unwrap_or_default();
    shelf.mark(&mut entries);
    let tracks: Vec<TitleTrack> = accounts
        .iter()
        .map(|a| {
            let entry = entries.iter().find(|e| e.platform == a.platform && e.item_id == title.item_id).cloned();
            let remote_id = entry.as_ref().map(|e| e.remote_id.clone()).unwrap_or_else(|| {
                title.remote_ids.get(&a.platform).and_then(|ids| ids.first().cloned()).unwrap_or_default()
            });
            TitleTrack {
                platform: a.platform.clone(),
                remote_id,
                matched_by: entry.as_ref().map(|e| e.matched_by.clone()).unwrap_or_default(),
                scrobble: a.scrobble,
                entry,
            }
        })
        .collect();
    Reply::ok(json!({"tracks": tracks}))
}

/// `PUT /api/list/{platform}/{remote_id}`: an edit written to the
/// platform, then to the stored copy (D-289). `item_id` describes a work
/// the list does not hold yet; a private library's title never goes
/// out.
fn entry_put(app: &App, req: &Req, v: &Verified) -> Reply {
    let patch: ListEntryPatch = match req.decode() {
        Ok(p) => p,
        Err(r) => return r,
    };
    let (platform, remote_id) = (req.param("platform"), req.param("remote_id"));
    let mut fallback = None;
    if !patch.item_id.is_empty() {
        let Some(item) = app.visible_item(v, &patch.item_id) else { return Reply::err(404, "unknown item") };
        if app.libraries.get(&item.library_id).is_some_and(|l| l.private) {
            return Reply::json(
                403,
                json!({"error": "a private library's titles stay on this server", "code": "private-library"}),
            );
        }
        let overlay = enrichments(app, std::slice::from_ref(&item.id)).into_iter().next();
        fallback = Some(ListEntry {
            title: overlay.as_ref().map(|o| o.title.clone()).filter(|t| !t.is_empty()).unwrap_or(item.title.clone()),
            cover: overlay.map(|o| o.poster).unwrap_or_default(),
            media_type: if is_reading(&item.kind) { list_media::MANGA } else { list_media::ANIME }.into(),
            ..ListEntry::default()
        });
    }
    match app.links.save_entry(&v.user_id, platform, remote_id, &patch, fallback) {
        Ok(mut entry) => {
            Shelf::read(app, v).mark(std::slice::from_mut(&mut entry));
            Reply::ok(json!({"entry": entry}))
        }
        Err(e) if e.code == "internal" => Reply::err(500, &e.msg),
        Err(e) if e.code == "not-found" => Reply::err(404, &e.msg),
        Err(e) => link_err(&e),
    }
}

fn links(app: &App, _: &Req, v: &Verified) -> Reply {
    match app.links.accounts(&v.user_id) {
        Ok(accounts) => {
            let views: Vec<_> = accounts.iter().map(|a| a.view(now())).collect();
            Reply::ok(json!({"links": views}))
        }
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

/// Starts an OAuth link: a signed, short-lived, user-bound state goes
/// into the platform's authorize URL (D-080).
fn authorize(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    let Some((client_id, _)) = app.links.client(platform) else { return not_configured("OAuth client", platform) };
    match app.links.authorize(&v.user_id, platform, &client_id, &callback_url(req, platform)) {
        Ok(url) => Reply::ok(json!({"url": url})),
        Err(e) if e.code == "internal" => Reply::err(500, &e.msg),
        Err(e) => link_err(&e),
    }
}

/// The zero-setup path (D-083): the official app's authorize URL; the
/// user pastes back the code AniList's pin page shows.
fn pin(app: &App, req: &Req, _: &Verified) -> Reply {
    let platform = req.param("platform");
    match app.links.pin_url(platform) {
        Some(url) => Reply::ok(json!({"url": url})),
        None => not_configured("pin client", platform),
    }
}

/// Exchanges the pasted pin code; a paste that fails never links.
fn pasted_code(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    if app.links.pin_url(platform).is_none() {
        return not_configured("pin client", platform);
    }
    #[derive(Deserialize)]
    struct In {
        #[serde(default)]
        code: String,
    }
    let i: In = match req.decode() {
        Ok(i) => i,
        Err(r) => return r,
    };
    let pasted = code_from_paste(&i.code);
    if pasted.is_empty() {
        return Reply::err(400, "code is required");
    }
    match app.links.exchange_pin(platform, &pasted) {
        Ok(id) => {
            let a = app.links.link(&v.user_id, platform, id, "pin");
            Reply::ok(json!({"link": a.view(now())}))
        }
        Err(e) if e.code == "internal" => Reply::err(500, &e.msg),
        Err(e) => link_err(&e),
    }
}

fn redirect(location: &str) -> Reply {
    let mut r = Response::new(Body::from(format!("<a href=\"{location}\">Found</a>.\n\n")));
    *r.status_mut() = StatusCode::FOUND;
    if let Ok(v) = HeaderValue::from_str(location) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    Reply::response(r)
}

/// The OAuth landing: public (no session header survives the
/// cross-site redirect), identity proven by the signed state. On
/// success the account is stored, the first import runs, and the
/// browser lands back on the account settings page.
fn callback(app: &App, req: &Req, _: &Verified) -> Reply {
    let platform = req.param("platform");
    let fail = |code: &str| redirect(&format!("/settings/connections?link_error={code}"));
    if !req.query("error").is_empty() {
        return fail("denied");
    }
    let code = req.query("code");
    let uid = match app.links.verify_state(req.query("state"), platform) {
        Some(uid) if !code.is_empty() => uid,
        _ => return Reply::err(400, "bad link state or code"),
    };
    // The state proves who asked; the account must still exist and be
    // enabled to complete a link.
    if !app.auth().get_public(&uid).is_some_and(|u| !u.disabled) {
        return Reply::err(400, "unknown or disabled user");
    }
    let Some((client_id, client_secret)) = app.links.client(platform) else { return fail(code::NOT_CONFIGURED) };
    let input = lain_lists::link::ExchangeInput {
        platform: platform.into(),
        client_id,
        client_secret,
        code: code.into(),
        redirect_uri: callback_url(req, platform),
    };
    match app.links.exchange(input) {
        Ok(id) => {
            app.links.link(&uid, platform, id, "oauth");
            redirect(&format!("/settings/connections?linked={platform}"))
        }
        Err(e) if e.code == "internal" => fail("exchange-failed"),
        Err(e) => fail(&e.code),
    }
}

fn link_patch(app: &App, req: &Req, v: &Verified) -> Reply {
    let bad = || Reply::err(400, "scrobble (boolean) required");
    let Ok(p) = req.decode::<lain_api::LinkPatch>() else { return bad() };
    let Some(on) = p.scrobble else { return bad() };
    match app.links.set_scrobble(&v.user_id, req.param("platform"), on) {
        Some(a) => Reply::ok(json!(a.view(now()))),
        None => not_linked(req.param("platform")),
    }
}

/// Unlinks: the token and the platform's imported entries (D-079).
fn unlink(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    match app.links.unlink(&v.user_id, platform) {
        Ok(Some(removed)) => Reply::ok(json!({"status": "ok", "removed": removed})),
        Ok(None) => not_linked(platform),
        Err(e) => Reply::err(503, &e.to_string()),
    }
}

fn sync(app: &App, req: &Req, v: &Verified) -> Reply {
    let platform = req.param("platform");
    let Some(a) = app.links.account(&v.user_id, platform) else { return not_linked(platform) };
    match app.links.sync(a) {
        Ok(stats) => Reply::ok(json!({"status": "ok", "stats": stats})),
        Err(e) => link_err(&e),
    }
}

// ---- operator integrations (D-080, D-198) ----------------------------------

fn integrations_get(app: &App, req: &Req, _: &Verified) -> Reply {
    Reply::ok(json!(integrations::view(&app.links.integrations(), &base_url(req))))
}

fn integrations_put(app: &App, req: &Req, _: &Verified) -> Reply {
    let u: integrations::IntegrationsUpdate = match req.decode() {
        Ok(u) => u,
        Err(r) => return r,
    };
    let cur = integrations::apply(app.links.integrations(), &u);
    if let Err(e) = app.links.save_integrations(&cur) {
        return Reply::err(500, &e.to_string());
    }
    tracing::info!("integrations settings updated");
    Reply::ok(json!(integrations::view(&cur, &base_url(req))))
}

// ---- scrobble ---------------------------------------------------------------

/// Queues a finished item for the platforms that opted in; the worker
/// writes off the request path. Items of a private library never leave
/// the server (D-156): scrobbling is an outward action about what the
/// user watched. The title's metadata names its platform ids and other
/// names, which the match prefers to the file's own title (D-288).
pub(crate) fn scrobble_after(app: &App, user_id: &str, it: &CatalogItem) {
    if app.libraries.get(&it.library_id).is_some_and(|l| l.private) {
        return;
    }
    if app.links.accounts(user_id).map_or(true, |a| !a.iter().any(|a| a.scrobble)) {
        return;
    }
    let group: Vec<CatalogItem> = app
        .registry
        .call_one(CAP_CATALOG_READ, "episodes", json!({"id": it.id}))
        .ok()
        .and_then(|(v, _)| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let mut ids: Vec<String> = vec![it.id.clone()];
    ids.extend(group.into_iter().map(|g| g.id).filter(|g| *g != it.id));
    let mut item = Finished {
        kind: it.kind.clone(),
        title: it.title.clone(),
        aliases: it.aliases.clone(),
        season: it.season,
        episode: it.episode,
        remote_ids: BTreeMap::new(),
    };
    // The file's own overlay first: an id it names wins over a sibling's.
    for o in enrichments(app, &ids) {
        if !o.title.is_empty() && !item.aliases.contains(&o.title) {
            item.aliases.push(o.title.clone());
        }
        if let Some(p) = platform_of(&o.provider)
            && !o.remote_id.is_empty()
        {
            item.remote_ids.entry(p.to_string()).or_insert(o.remote_id);
        }
    }
    app.links.enqueue(user_id, item);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str, kind: &str, title: &str, season: i64, episode: i64) -> CatalogItem {
        CatalogItem { id: id.into(), kind: kind.into(), title: title.into(), season, episode, ..CatalogItem::default() }
    }

    fn overlay(item: &str, provider: &str, remote: &str, title: &str) -> Enrichment {
        Enrichment {
            item_id: item.into(),
            provider: provider.into(),
            remote_id: remote.into(),
            title: title.into(),
            ..Enrichment::default()
        }
    }

    fn entry(rid: &str, media: &str, title: &str) -> ListEntry {
        ListEntry {
            platform: "anilist".into(),
            remote_id: rid.into(),
            media_type: media.into(),
            title: title.into(),
            ..ListEntry::default()
        }
    }

    #[test]
    fn entries_find_their_library_title() {
        let items = vec![
            file("f2", "episode", "Sousou no Frieren", 1, 2),
            file("f1", "episode", "Sousou no Frieren", 1, 1),
            file("b1", "episode", "Bocchi the Rock", 1, 1),
            file("m1", "manga", "One Piece", 1, 1),
            file("x1", "episode", "Twin", 1, 1),
            CatalogItem { aliases: vec!["Twin".into()], ..file("y1", "episode", "Other", 1, 1) },
        ];
        let overlays = vec![
            overlay("f2", ANILIST_ID, "154587", "Frieren: Beyond Journey's End"),
            overlay("b1", "lain-metadata-kitsu", "999", "Bocchi the Rock!"),
        ];
        let shelf = Shelf::build(items, &overlays);
        let mut list = vec![
            entry("154587", "anime", "Some English Name"),
            entry("130003", "anime", "BOCCHI THE ROCK!"),
            entry("30013", "manga", "One Piece"),
            entry("21", "anime", "One Piece"),
            entry("5", "anime", "Twin"),
            ListEntry { alt_titles: vec!["Frieren: Beyond Journey's End".into()], ..entry("1", "anime", "Unrelated") },
        ];
        shelf.mark(&mut list);
        let got: Vec<(&str, &str)> = list.iter().map(|e| (e.item_id.as_str(), e.matched_by.as_str())).collect();
        assert_eq!(
            got,
            [
                ("f1", "id"),    // the metadata's id, on the title's first file
                ("b1", "title"), // a Kitsu id is not AniList's: the names match
                ("m1", "title"), // manga to a reading title
                ("", ""),        // an anime entry never to a manga title
                ("", ""),        // two titles go by that name: not guessed
                ("f1", "title"), // an alt title against the overlay's name
            ]
        );
    }
}
