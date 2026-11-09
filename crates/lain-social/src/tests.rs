//! The Go provider tests' intent (`internal/plugins/social/social_test.go`),
//! plus D-156: rows naming an item of a hidden library never show.

use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::json;

use super::*;
use lain_api::{SocialSettingsPatch, activity_type as at, notify, relation as rel, visibility as vis};

struct T {
    s: Social,
    clock: Arc<AtomicI64>,
    dir: std::path::PathBuf,
}

impl T {
    fn new() -> T {
        let dir = lain_store::test_dir("social");
        let s = Social::new(Arc::new(Store::open(&dir).unwrap()));
        let clock = Arc::new(AtomicI64::new(1_800_000_000));
        let c = clock.clone();
        s.set_clock(Arc::new(move || c.load(Ordering::SeqCst) * 1_000_000_000));
        T { s, clock, dir }
    }
    fn advance(&self, secs: i64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
    }
    fn befriend(&self, a: &str, b: &str) {
        self.s.relate(a, b, ACTION_REQUEST).unwrap();
        assert_eq!(self.s.relate(b, a, ACTION_ACCEPT).unwrap().state, rel::FRIEND);
    }
    fn notes(&self, u: &str) -> NotificationsOutput {
        self.s.notifications(u, 0, &Hidden::new()).unwrap()
    }
}

impl Drop for T {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn code<V: std::fmt::Debug>(r: Result<V, Error>) -> String {
    match r {
        Ok(v) => format!("ok: {v:?}"),
        Err(e) => e.code,
    }
}

fn work(kind: &str, title: &str) -> WorkRef {
    WorkRef { kind: kind.into(), title: title.into(), ..WorkRef::default() }
}

fn frieren() -> WorkRef {
    work("manga", "Frieren")
}

fn none() -> Hidden {
    Hidden::new()
}

fn patch() -> SocialSettingsPatch {
    SocialSettingsPatch::default()
}

#[test]
fn normalize_work_is_media_agnostic() {
    for kind in ["anime", "series", "movie", "comic", "manga", "light-novel"] {
        let w = normalize_work(work(&format!(" {} ", kind.to_uppercase()), "  One   Piece ")).unwrap();
        assert_eq!((w.kind.as_str(), w.title.as_str()), (kind, "One Piece"));
    }
    let a = normalize_work(work("manga", "One Piece")).unwrap();
    let b = normalize_work(work("manga", "ONE  piece")).unwrap();
    let c = normalize_work(work("anime", "One Piece")).unwrap();
    assert_eq!(work_key(&a), work_key(&b));
    assert_ne!(work_key(&a), work_key(&c));
    for bad in [
        work("", "x"),
        work("ma nga", "x"),
        work("manga", "  "),
        work("manga", "a\0b"),
        work("manga", &"x".repeat(201)),
    ] {
        assert_eq!(code(normalize_work(bad.clone())), "invalid-message", "{bad:?}");
    }
    let w =
        normalize_work(WorkRef { item_id: "../etc".into(), library_id: "lib".into(), ..work("anime", "x") }).unwrap();
    assert_eq!(
        (w.item_id.as_str(), w.library_id.as_str()),
        ("", ""),
        "a path-like item id is dropped with its library"
    );
}

#[test]
fn friend_lifecycle() {
    let t = T::new();
    let s = &t.s;
    assert_eq!(s.relate("ana", "bo", ACTION_REQUEST).unwrap().state, rel::OUTGOING);
    assert_eq!(s.relation("bo", "ana").unwrap().state, rel::INCOMING);
    // Requesting twice is idempotent and does not notify twice.
    s.relate("ana", "bo", ACTION_REQUEST).unwrap();
    let n = t.notes("bo");
    assert_eq!(
        (n.unread, n.items[0].kind.as_str(), n.items[0].from_user_id.as_str()),
        (1, notify::FRIEND_REQUEST, "ana")
    );
    // Accepting is a request in the other direction.
    assert_eq!(s.relate("bo", "ana", ACTION_REQUEST).unwrap().state, rel::FRIEND);
    let n = t.notes("ana");
    assert_eq!((n.unread, n.items[0].kind.as_str()), (1, notify::FRIEND_ACCEPTED));
    let r = s.relations("ana").unwrap();
    assert_eq!((r.len(), r[0].user_id.as_str(), r[0].state.as_str()), (1, "bo", rel::FRIEND));
    assert_eq!(s.relate("bo", "ana", ACTION_REMOVE).unwrap().state, rel::NONE);
    assert_eq!(s.relation("ana", "bo").unwrap().state, rel::NONE);
    s.relate("ana", "bo", ACTION_REQUEST).unwrap();
    assert_eq!(s.relate("bo", "ana", ACTION_DECLINE).unwrap().state, rel::NONE);
    assert_eq!(code(s.relate("bo", "ana", ACTION_DECLINE)), "not-found");
    assert_eq!(code(s.relate("bo", "ana", ACTION_ACCEPT)), "not-found");
    s.relate("ana", "bo", ACTION_REQUEST).unwrap();
    s.relate("ana", "bo", ACTION_REMOVE).unwrap();
    assert_eq!(s.relation("bo", "ana").unwrap().state, rel::NONE);
    assert_eq!(code(s.relate("ana", "ana", ACTION_REQUEST)), "invalid-message");
    assert_eq!(code(s.relate("ana", "bo", "poke")), "invalid-message");
}

#[test]
fn answered_requests_leave_the_inbox() {
    let t = T::new();
    let s = &t.s;
    let pending = |u: &str| t.notes(u).items.iter().filter(|n| n.kind == notify::FRIEND_REQUEST && !n.read).count();
    s.relate("ana", "bo", ACTION_REQUEST).unwrap();
    s.relate("bo", "ana", ACTION_ACCEPT).unwrap();
    assert_eq!(pending("bo"), 0, "accepted");
    s.relate("bo", "ana", ACTION_REMOVE).unwrap();
    s.relate("ana", "bo", ACTION_REQUEST).unwrap();
    s.relate("bo", "ana", ACTION_DECLINE).unwrap();
    assert_eq!(pending("bo"), 0, "declined");
    s.relate("ana", "bo", ACTION_REQUEST).unwrap();
    s.relate("ana", "bo", ACTION_REMOVE).unwrap();
    assert_eq!(pending("bo"), 0, "cancelled");
}

#[test]
fn requests_can_be_closed() {
    let t = T::new();
    t.s.patch_settings("bo", SocialSettingsPatch { allow_requests: Some("nobody".into()), ..patch() }).unwrap();
    assert_eq!(code(t.s.relate("ana", "bo", ACTION_REQUEST)), "forbidden");
    t.befriend("bo", "ana");
}

#[test]
fn block_is_symmetric_and_silent() {
    let t = T::new();
    let s = &t.s;
    t.befriend("ana", "bo");
    assert_eq!(s.relate("ana", "bo", ACTION_BLOCK).unwrap().state, rel::BLOCKING);
    let r = s.relation("bo", "ana").unwrap();
    assert_eq!((r.state.as_str(), r.blocked_by), (rel::NONE, true));
    assert!(s.relations("bo").unwrap().is_empty(), "bo's list never reveals the block");
    assert_eq!(code(s.relate("bo", "ana", ACTION_REQUEST)), "not-found");
    assert_eq!(code(s.relate("ana", "bo", ACTION_REQUEST)), "forbidden");
    let public =
        SocialSettingsPatch { activity: Some(vis::PUBLIC.into()), ratings: Some(vis::PUBLIC.into()), ..patch() };
    s.patch_settings("ana", public.clone()).unwrap();
    s.patch_settings("bo", public).unwrap();
    for (a, b) in [("ana", "bo"), ("bo", "ana")] {
        assert!(!s.can_see(a, b, SUBJECT_PROFILE).unwrap(), "{a} sees {b} across a block");
    }
    s.relate("bo", "ana", ACTION_BLOCK).unwrap();
    s.relate("ana", "bo", ACTION_UNBLOCK).unwrap();
    let r = s.relation("ana", "bo").unwrap();
    assert_eq!((r.state.as_str(), r.blocked_by), (rel::NONE, true));
    s.relate("bo", "ana", ACTION_UNBLOCK).unwrap();
    let r = s.relation("ana", "bo").unwrap();
    assert_eq!((r.state.as_str(), r.blocked_by), (rel::NONE, false));
    t.befriend("ana", "bo");
}

#[test]
fn privacy_matrix() {
    let t = T::new();
    let s = &t.s;
    t.befriend("owner", "friend");
    for (level, want) in [
        (vis::PUBLIC, [("owner", true), ("friend", true), ("stranger", true)]),
        (vis::FRIENDS, [("owner", true), ("friend", true), ("stranger", false)]),
        (vis::PRIVATE, [("owner", true), ("friend", false), ("stranger", false)]),
    ] {
        let l = Some(level.to_string());
        s.patch_settings(
            "owner",
            SocialSettingsPatch { profile: l.clone(), activity: l.clone(), ratings: l, ..patch() },
        )
        .unwrap();
        for (viewer, ok) in want {
            for subj in [SUBJECT_PROFILE, SUBJECT_ACTIVITY, SUBJECT_RATINGS] {
                assert_eq!(s.can_see(viewer, "owner", subj).unwrap(), ok, "{level}/{viewer}/{subj}");
            }
        }
    }
    assert_eq!(
        code(s.patch_settings("owner", SocialSettingsPatch { profile: Some("everyone".into()), ..patch() })),
        "invalid-message"
    );
    assert_eq!(code(s.can_see("friend", "owner", "diary")), "invalid-message");
}

#[test]
fn settings_defaults_and_favorites() {
    let t = T::new();
    let s = &t.s;
    let st = s.settings("ana", &none()).unwrap();
    assert_eq!(st, default_settings("ana"));
    assert_eq!((st.profile.as_str(), st.activity.as_str(), st.ratings.as_str()), ("public", "friends", "friends"));
    assert!(st.discoverable && st.allow_requests == "everyone");
    let favs = vec![frieren(), work("MANGA", "frieren "), work("comic", "Saga")];
    let st = s.patch_settings("ana", SocialSettingsPatch { favorites: Some(favs), ..patch() }).unwrap();
    assert_eq!(st.favorites.len(), 2);
    assert_eq!(st.favorites[1].kind, "comic");
    let st = s.patch_settings("ana", SocialSettingsPatch { discoverable: Some(false), ..patch() }).unwrap();
    assert!(st.favorites.len() == 2 && !st.discoverable, "a patch without favorites keeps them");
    let many = (0..13).map(|i| work("anime", &"x".repeat(i + 1))).collect();
    assert_eq!(
        code(s.patch_settings("ana", SocialSettingsPatch { favorites: Some(many), ..patch() })),
        "invalid-message"
    );
}

#[test]
fn progress_activity_is_deduplicated() {
    let t = T::new();
    let rec = |ep: i64, done: bool, was: bool| {
        t.s.record_progress(RecordProgressInput {
            user_id: "ana".into(),
            work: WorkRef { item_id: "abc".into(), ..work("anime", "Frieren") },
            episode: ep,
            completed: done,
            was_completed: was,
            ..RecordProgressInput::default()
        })
        .unwrap()
        .recorded
    };
    assert_eq!(rec(1, false, false), at::PROGRESS);
    t.advance(60);
    assert_eq!(rec(1, false, false), "");
    assert_eq!(rec(1, true, false), at::COMPLETED);
    assert_eq!(rec(1, true, true), "");
    t.advance(PROGRESS_WINDOW_SECS);
    assert_eq!(rec(2, false, false), at::PROGRESS);
    let rows =
        t.s.user_activity(&UserActivityInput { viewer_id: "ana".into(), owner_id: "ana".into(), ..Default::default() })
            .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!((rows[0].episode, rows[1].kind.as_str()), (2, at::COMPLETED));
}

#[test]
fn activity_is_capped() {
    let t = T::new();
    for _ in 0..MAX_ACTIVITY_ROWS + 20 {
        t.advance(1);
        t.s.record_progress(RecordProgressInput {
            user_id: "ana".into(),
            work: work("comic", "Saga"),
            completed: true,
            ..Default::default()
        })
        .unwrap();
    }
    let page = |before: i64| {
        t.s.user_activity(&UserActivityInput {
            viewer_id: "ana".into(),
            owner_id: "ana".into(),
            before,
            limit: MAX_PAGE_LIMIT as i64,
            ..Default::default()
        })
        .unwrap()
    };
    assert_eq!(page(0).len(), MAX_PAGE_LIMIT);
    let (mut n, mut before) = (0, 0);
    loop {
        let p = page(before);
        if p.is_empty() {
            break;
        }
        n += p.len();
        before = p.last().unwrap().at;
    }
    assert_eq!(n, MAX_ACTIVITY_ROWS);
}

#[test]
fn feed_respects_privacy() {
    let t = T::new();
    let s = &t.s;
    for f in ["open", "quiet", "shy"] {
        t.befriend("me", f);
    }
    s.patch_settings("quiet", SocialSettingsPatch { activity: Some(vis::PRIVATE.into()), ..patch() }).unwrap();
    s.patch_settings(
        "shy",
        SocialSettingsPatch { hide_progress: Some(true), ratings: Some(vis::PRIVATE.into()), ..patch() },
    )
    .unwrap();
    for u in ["open", "quiet", "shy", "stranger"] {
        t.advance(1);
        s.record_progress(RecordProgressInput { user_id: u.into(), work: frieren(), ..Default::default() }).unwrap();
        t.advance(1);
        s.record_progress(RecordProgressInput {
            user_id: u.into(),
            work: frieren(),
            completed: true,
            ..Default::default()
        })
        .unwrap();
        t.advance(1);
        s.put_rating(PutRatingInput { user_id: u.into(), work: frieren(), score: 9, ..Default::default() }).unwrap();
    }
    let feed = s.feed(&FeedInput { user_id: "me".into(), ..Default::default() }).unwrap();
    let of = |u: &str| feed.iter().filter(|a| a.user_id == u).map(|a| a.kind.as_str()).collect::<Vec<_>>().join(",");
    assert_eq!(of("open"), "rated,completed,progress");
    assert_eq!((of("quiet"), of("stranger"), of("me")), (String::new(), String::new(), String::new()));
    assert_eq!(of("shy"), "completed", "progress hidden, ratings private");
    assert!(feed.windows(2).all(|w| w[0].at >= w[1].at), "newest first");
    let page = s.feed(&FeedInput { user_id: "me".into(), limit: 2, ..Default::default() }).unwrap();
    let rest = s.feed(&FeedInput { user_id: "me".into(), before: page[1].at, ..Default::default() }).unwrap();
    assert_eq!((page.len(), rest.len()), (2, feed.len() - 2));
}

#[test]
fn ratings_and_reviews() {
    let t = T::new();
    let s = &t.s;
    t.befriend("ana", "bo");
    let r = s
        .put_rating(PutRatingInput {
            user_id: "ana".into(),
            work: frieren(),
            score: 8,
            review: "  quiet\nand kind  ".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((r.score, r.review.as_str()), (8, "quiet\nand kind"));
    let created = r.created_at;
    t.advance(3600);
    let r = s
        .put_rating(PutRatingInput {
            user_id: "ana".into(),
            work: work("manga", "FRIEREN"),
            score: 10,
            ..Default::default()
        })
        .unwrap();
    assert!(r.created_at == created && r.updated_at != created, "an update keeps created_at");
    s.put_rating(PutRatingInput { user_id: "bo".into(), work: frieren(), score: 6, ..Default::default() }).unwrap();
    s.put_rating(PutRatingInput { user_id: "cy".into(), work: frieren(), score: 1, ..Default::default() }).unwrap();
    let v = s.work("ana", frieren(), &none()).unwrap();
    assert_eq!(v.mine.as_ref().map(|m| m.score), Some(10));
    assert_eq!((v.ratings.len(), v.ratings[0].user_id.as_str()), (1, "bo"));
    assert_eq!((v.scored, v.average), (2, 8.0), "the average counts only visible scores");
    let anime = s.work("ana", work("anime", "Frieren"), &none()).unwrap();
    assert!(anime.mine.is_none() && anime.ratings.is_empty());
    let list = s.user_ratings("bo", "ana", &none()).unwrap();
    assert_eq!((list.len(), list[0].score), (1, 10));
    assert!(s.user_ratings("cy", "ana", &none()).unwrap().is_empty());
    for (score, review) in [(11, ""), (-1, ""), (0, ""), (5, &*"x".repeat(2001))] {
        let i = PutRatingInput {
            user_id: "ana".into(),
            work: frieren(),
            score,
            review: review.into(),
            ..Default::default()
        };
        assert_eq!(code(s.put_rating(i)), "invalid-message", "{score}");
    }
    s.put_rating(PutRatingInput {
        user_id: "bo".into(),
        work: work("comic", "Saga"),
        review: "yes".into(),
        ..Default::default()
    })
    .unwrap();
    assert!(s.delete_rating("ana", frieren()).unwrap());
    assert!(s.work("ana", frieren(), &none()).unwrap().mine.is_none());
    assert!(s.user_ratings("ana", "ana", &none()).unwrap().is_empty(), "the by-user index follows the delete");
}

#[test]
fn comments_replies_and_moderation() {
    let t = T::new();
    let s = &t.s;
    let add = |u: &str, w: WorkRef, body: &str, reply: &str| {
        s.add_comment(AddCommentInput { user_id: u.into(), work: w, body: body.into(), reply_to: reply.into() })
    };
    let top = add("ana", frieren(), "chapter 60 tho", "").unwrap();
    let reply = add("bo", frieren(), "yes", &top.id).unwrap();
    assert_eq!(reply.reply_to, top.id);
    let n = t.notes("ana");
    assert_eq!(
        (n.unread, n.items[0].kind.as_str(), n.items[0].comment_id.as_str()),
        (1, notify::REPLY, reply.id.as_str())
    );
    add("ana", frieren(), "and 61", &top.id).unwrap();
    assert_eq!(t.notes("ana").unread, 1, "a self reply notifies nobody");
    assert_eq!(code(add("bo", work("anime", "Frieren"), "x", &top.id)), "not-found");
    assert_eq!(code(add("bo", frieren(), "   ", "")), "invalid-message");
    let v = s.work("cy", frieren(), &none()).unwrap();
    assert_eq!((v.comments.len(), v.comments[0].id.as_str()), (3, top.id.as_str()));
    s.relate("cy", "bo", ACTION_BLOCK).unwrap();
    assert_eq!(s.work("cy", frieren(), &none()).unwrap().comments.len(), 2, "blocked authors vanish");
    let del = |u: &str, m: bool| {
        s.delete_comment(&DeleteCommentInput { user_id: u.into(), id: top.id.clone(), moderator: m })
    };
    assert_eq!(code(del("cy", false)), "forbidden");
    del("cy", true).unwrap();
    assert_eq!(code(del("ana", false)), "not-found");
}

fn change(user: &str, id: &str) -> ChangeCollectionInput {
    ChangeCollectionInput { user_id: user.into(), id: id.into(), ..Default::default() }
}

#[test]
fn collections_visibility_and_sharing() {
    let t = T::new();
    let s = &t.s;
    t.befriend("ana", "bo");
    let col = s
        .create_collection(CreateCollectionInput {
            user_id: "ana".into(),
            name: "  Cozy   reads ".into(),
            description: "for rain".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((col.name.as_str(), col.visibility.as_str()), ("Cozy reads", "private"));
    for w in [frieren(), work("comic", "Saga"), work("movie", "Perfect Blue")] {
        s.add_collection_item(ChangeCollectionInput { work: Some(w), ..change("ana", &col.id) }).unwrap();
    }
    let c = s
        .add_collection_item(ChangeCollectionInput {
            work: Some(work("manga", "frieren")),
            note: "start here".into(),
            ..change("ana", &col.id)
        })
        .unwrap();
    assert_eq!((c.items.len(), c.items[0].note.as_str()), (3, "start here"), "a re-add updates the note in place");
    assert_eq!(code(s.collection("bo", &col.id, &none())), "not-found");
    assert_eq!(
        code(s.add_collection_item(ChangeCollectionInput { work: Some(frieren()), ..change("bo", &col.id) })),
        "not-found"
    );
    assert_eq!(
        code(s.share_collection(ChangeCollectionInput { to: vec!["cy".into()], ..change("ana", &col.id) })),
        "forbidden"
    );
    let out = s
        .share_collection(ChangeCollectionInput {
            to: vec!["bo".into(), "bo".into(), "ana".into()],
            message: "rainy day".into(),
            ..change("ana", &col.id)
        })
        .unwrap();
    assert_eq!(out.sent, vec!["bo".to_string()]);
    let got = s.collection("bo", &col.id, &none()).unwrap();
    assert!(got.shared_with.is_empty(), "bo does not see the share list");
    assert_eq!(
        code(s.add_collection_item(ChangeCollectionInput { work: Some(frieren()), ..change("bo", &col.id) })),
        "forbidden"
    );
    let mine = s.collections("bo", "", &none()).unwrap();
    assert_eq!((mine.len(), mine[0].id.as_str()), (1, col.id.as_str()));
    let n = &t.notes("bo").items[0];
    assert_eq!((n.kind.as_str(), n.message.as_str()), (notify::COLLECTION_SHARED, "rainy day"));
    s.relate("bo", "ana", ACTION_REMOVE).unwrap();
    assert_eq!(code(s.collection("bo", &col.id, &none())), "not-found", "unfriending revokes the share");
    let mut public = change("ana", &col.id);
    public.patch.visibility = Some(vis::PUBLIC.into());
    s.update_collection(public).unwrap();
    assert_eq!(s.collections("cy", "ana", &none()).unwrap().len(), 1);
    s.patch_settings("ana", SocialSettingsPatch { profile: Some(vis::PRIVATE.into()), ..patch() }).unwrap();
    assert!(s.collections("cy", "ana", &none()).unwrap().is_empty(), "the profile gate covers the list");
    let c =
        s.remove_collection_item(ChangeCollectionInput { work: Some(frieren()), ..change("ana", &col.id) }).unwrap();
    assert_eq!(c.items.len(), 2);
    assert_eq!(code(s.delete_collection("cy", &col.id)), "not-found");
    assert!(s.delete_collection("ana", &col.id).unwrap());
    assert_eq!(
        code(s.create_collection(CreateCollectionInput {
            user_id: "ana".into(),
            name: " ".into(),
            ..Default::default()
        })),
        "invalid-message"
    );
}

#[test]
fn share_work_and_notifications() {
    let t = T::new();
    let s = &t.s;
    t.befriend("ana", "bo");
    t.befriend("ana", "cy");
    let share = |to: &[&str]| {
        s.share_work(ShareWorkInput {
            user_id: "ana".into(),
            to: to.iter().map(|x| x.to_string()).collect(),
            work: frieren(),
            message: "read this".into(),
        })
    };
    assert_eq!(code(share(&["dee"])), "forbidden");
    assert_eq!(code(share(&[])), "invalid-message");
    assert_eq!(share(&["bo", "cy"]).unwrap().sent.len(), 2);
    let n = t.notes("bo");
    let row = n.items[0].clone();
    assert_eq!(
        (row.kind.as_str(), row.work.as_ref().map(|w| w.title.as_str()), row.message.as_str()),
        (notify::SHARE, Some("Frieren"), "read this")
    );
    let before = n.unread;
    assert_eq!(s.mark_read(&MarkReadInput { user_id: "bo".into(), ids: vec![row.id.clone()], all: false }).unwrap(), 1);
    assert_eq!(t.notes("bo").unread, before - 1);
    s.mark_read(&MarkReadInput { user_id: "bo".into(), all: true, ..Default::default() }).unwrap();
    assert_eq!(t.notes("bo").unread, 0);
    s.relate("cy", "ana", ACTION_BLOCK).unwrap();
    assert!(t.notes("cy").items.is_empty(), "blocking the sender hides old rows");
}

#[test]
fn notifications_are_capped_and_announced() {
    let t = T::new();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = seen.clone();
    t.s.on_notify(Arc::new(move |n: &Notification| {
        sink.lock().unwrap().push(format!("{}>{}", n.from_user_id, n.user_id))
    }));
    t.befriend("ana", "bo");
    assert_eq!(*seen.lock().unwrap(), vec!["ana>bo", "bo>ana"], "request and acceptance are announced");
    for _ in 0..MAX_NOTIFICATIONS + 10 {
        t.advance(1);
        t.s.share_work(ShareWorkInput {
            user_id: "ana".into(),
            to: vec!["bo".into()],
            work: frieren(),
            message: String::new(),
        })
        .unwrap();
    }
    let n = t.s.notifications("bo", MAX_PAGE_LIMIT as i64, &none()).unwrap();
    assert_eq!(n.unread, MAX_NOTIFICATIONS as i64);
    // A failed transaction announces nothing.
    let before = seen.lock().unwrap().len();
    assert!(
        t.s.share_work(ShareWorkInput {
            user_id: "ana".into(),
            to: vec!["bo".into(), "zed".into()],
            work: frieren(),
            message: String::new()
        })
        .is_err()
    );
    assert_eq!(seen.lock().unwrap().len(), before);
}

#[test]
fn invoke_rejects_wrong_inputs() {
    let t = T::new();
    for cap in CAPABILITIES {
        assert_eq!(code(t.s.invoke(cap, "nope", json!({}))), "invalid-message", "{cap}");
    }
    assert_eq!(code(t.s.invoke(CAP_SOCIAL_GRAPH, graph_op::SETTINGS, json!({}))), "invalid-message");
    assert_eq!(code(t.s.invoke(CAP_SOCIAL_GRAPH, graph_op::SETTINGS, json!({"user_id": 5}))), "invalid-message");
    assert_eq!(code(t.s.invoke("lain.social.nope@1", "x", json!({}))), "invalid-message");
    let out =
        t.s.invoke(CAP_SOCIAL_GRAPH, graph_op::RELATE, json!({"user_id": "a", "other_id": "b", "action": "request"}))
            .unwrap();
    assert_eq!(out, json!({"state": "outgoing", "blocked_by": false}));
}

/// D-156: rows whose work names an item of a hidden library never show
/// (feed, profile activity, ratings, the work view and its average,
/// comments, collections, favorites, notifications), and the visible
/// ones are counted and paged without them.
#[test]
fn hidden_libraries_never_leak() {
    let t = T::new();
    let s = &t.s;
    t.befriend("ana", "bo");
    let public =
        SocialSettingsPatch { activity: Some(vis::PUBLIC.into()), ratings: Some(vis::PUBLIC.into()), ..patch() };
    s.patch_settings("ana", public).unwrap();
    let secret = WorkRef { item_id: "item-x".into(), library_id: "lib-private".into(), ..work("anime", "Secret") };
    let open = WorkRef { item_id: "item-o".into(), library_id: "lib-public".into(), ..work("anime", "Open") };
    for w in [&secret, &open] {
        t.advance(1);
        s.record_progress(RecordProgressInput { user_id: "ana".into(), work: w.clone(), ..Default::default() })
            .unwrap();
        s.put_rating(PutRatingInput { user_id: "ana".into(), work: w.clone(), score: 7, ..Default::default() })
            .unwrap();
        s.put_rating(PutRatingInput { user_id: "bo".into(), work: w.clone(), score: 3, ..Default::default() }).unwrap();
        s.add_comment(AddCommentInput {
            user_id: "ana".into(),
            work: w.clone(),
            body: "hm".into(),
            reply_to: String::new(),
        })
        .unwrap();
    }
    let hid: Hidden = ["lib-private".to_string()].into();
    let no_secret = |ws: Vec<&WorkRef>| ws.iter().all(|w| w.title != "Secret");

    let feed = s
        .feed(&FeedInput { user_id: "bo".into(), hidden_libraries: vec!["lib-private".into()], ..Default::default() })
        .unwrap();
    assert!(!feed.is_empty() && no_secret(feed.iter().map(|a| &a.work).collect()), "feed: {feed:?}");
    let page = s
        .feed(&FeedInput {
            user_id: "bo".into(),
            limit: 1,
            hidden_libraries: vec!["lib-private".into()],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(page.len(), 1, "paging counts visible rows only");
    for viewer in ["bo", "ana"] {
        let rows = s
            .user_activity(&UserActivityInput {
                viewer_id: viewer.into(),
                owner_id: "ana".into(),
                hidden_libraries: vec!["lib-private".into()],
                ..Default::default()
            })
            .unwrap();
        assert!(!rows.is_empty() && no_secret(rows.iter().map(|a| &a.work).collect()), "{viewer}: {rows:?}");
        let rs = s.user_ratings(viewer, "ana", &hid).unwrap();
        assert_eq!(rs.len(), 1, "{viewer}: {rs:?}");
        assert_eq!(rs[0].work.title, "Open");
    }
    // The work view: the owner's own rating too, and the average.
    let v = s.work("ana", work("anime", "Secret"), &hid).unwrap();
    assert!(v.mine.is_none() && v.ratings.is_empty() && v.comments.is_empty() && v.scored == 0, "{v:?}");
    let v = s.work("ana", work("anime", "Secret"), &none()).unwrap();
    assert_eq!((v.scored, v.average, v.comments.len()), (2, 5.0, 1), "an unlocked session sees all");

    // Collections, favorites and notifications.
    let col = s
        .create_collection(CreateCollectionInput {
            user_id: "ana".into(),
            name: "mix".into(),
            visibility: "public".into(),
            ..Default::default()
        })
        .unwrap();
    for w in [&secret, &open] {
        s.add_collection_item(ChangeCollectionInput { work: Some(w.clone()), ..change("ana", &col.id) }).unwrap();
    }
    let c = s.collection("bo", &col.id, &hid).unwrap();
    assert_eq!(c.items.len(), 1);
    assert!(s.collections("ana", "", &hid).unwrap()[0].items.iter().all(|i| i.work.title == "Open"));
    let c = s
        .remove_collection_item(ChangeCollectionInput {
            work: Some(work("comic", "none")),
            hidden_libraries: vec!["lib-private".into()],
            ..change("ana", &col.id)
        })
        .unwrap();
    assert_eq!(c.items.len(), 1, "a mutation answers without hidden works");
    assert_eq!(s.collection("ana", &col.id, &none()).unwrap().items.len(), 2, "and keeps them stored");
    s.patch_settings("ana", SocialSettingsPatch { favorites: Some(vec![secret.clone(), open.clone()]), ..patch() })
        .unwrap();
    assert_eq!(s.settings("ana", &hid).unwrap().favorites.len(), 1);
    let out = s
        .invoke(
            CAP_SOCIAL_GRAPH,
            graph_op::PATCH_SETTINGS,
            json!({"user_id": "ana", "patch": {}, "hidden_libraries": ["lib-private"]}),
        )
        .unwrap();
    assert_eq!(out["favorites"].as_array().unwrap().len(), 1);
    s.share_work(ShareWorkInput {
        user_id: "ana".into(),
        to: vec!["bo".into()],
        work: secret.clone(),
        message: String::new(),
    })
    .unwrap();
    let n = s.notifications("bo", 0, &hid).unwrap();
    assert!(n.items.iter().all(|n| n.work.is_none()) && n.unread == 0, "only the answered request remains: {n:?}");
    assert_eq!(s.notifications("bo", 0, &none()).unwrap().unread, 1);
}
