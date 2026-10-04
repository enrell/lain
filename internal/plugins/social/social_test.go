package social

import (
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/testutil/contract"
)

type clock struct{ t time.Time }

func (c *clock) now() time.Time          { return c.t }
func (c *clock) advance(d time.Duration) { c.t = c.t.Add(d) }

func newService(t *testing.T) (*Service, *clock) {
	t.Helper()
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	c := &clock{t: time.Unix(1_800_000_000, 0)}
	s.SetClock(c.now)
	return s, c
}

func code(err error) string {
	if e, ok := err.(*core.Error); ok {
		return e.Code
	}
	if err == nil {
		return ""
	}
	return "untyped: " + err.Error()
}

func befriend(t *testing.T, s *Service, a, b string) {
	t.Helper()
	if _, err := s.Relate(a, b, ActionRequest); err != nil {
		t.Fatal(err)
	}
	if r, err := s.Relate(b, a, ActionAccept); err != nil || r.State != contracts.RelationFriend {
		t.Fatalf("accept: %+v %v", r, err)
	}
}

var frieren = contracts.WorkRef{Kind: "manga", Title: "Frieren"}

func TestProviderContract(t *testing.T) {
	s, _ := newService(t)
	contract.Run(t, s, []contract.Cap{
		{Name: contracts.CapSocialGraph, Sample: GetSettingsInput{UserID: "u1"}},
		{Name: contracts.CapSocialActivity, Sample: FeedInput{UserID: "u1"}},
		{Name: contracts.CapSocialReviews, Sample: WorkInput{ViewerID: "u1", Work: frieren}},
		{Name: contracts.CapSocialCollections, Sample: ListCollectionsInput{ViewerID: "u1"}},
	})
}

func TestNormalizeWorkIsMediaAgnostic(t *testing.T) {
	for _, kind := range []string{"anime", "series", "movie", "comic", "manga", "light-novel"} {
		w, err := NormalizeWork(contracts.WorkRef{Kind: " " + strings.ToUpper(kind) + " ", Title: "  One   Piece "})
		if err != nil || w.Kind != kind || w.Title != "One Piece" {
			t.Fatalf("%s: %+v %v", kind, w, err)
		}
	}
	a, _ := NormalizeWork(contracts.WorkRef{Kind: "manga", Title: "One Piece"})
	b, _ := NormalizeWork(contracts.WorkRef{Kind: "manga", Title: "ONE  piece"})
	c, _ := NormalizeWork(contracts.WorkRef{Kind: "anime", Title: "One Piece"})
	if WorkKey(a) != WorkKey(b) {
		t.Fatal("same work under different spelling must share a key")
	}
	if WorkKey(a) == WorkKey(c) {
		t.Fatal("manga and anime of one title must stay distinct works")
	}
	for _, bad := range []contracts.WorkRef{
		{Kind: "", Title: "x"}, {Kind: "ma nga", Title: "x"}, {Kind: "manga", Title: "  "},
		{Kind: "manga", Title: "a\x00b"}, {Kind: "manga", Title: strings.Repeat("x", 201)},
	} {
		if _, err := NormalizeWork(bad); code(err) != "invalid-message" {
			t.Errorf("%+v: %v", bad, err)
		}
	}
	if w, _ := NormalizeWork(contracts.WorkRef{Kind: "anime", Title: "x", ItemID: "../etc"}); w.ItemID != "" {
		t.Fatal("a path-like item id must be dropped")
	}
}

func TestFriendLifecycle(t *testing.T) {
	s, _ := newService(t)
	r, err := s.Relate("ana", "bo", ActionRequest)
	if err != nil || r.State != contracts.RelationOutgoing {
		t.Fatalf("request: %+v %v", r, err)
	}
	if r, _ := s.Relation("bo", "ana"); r.State != contracts.RelationIncoming {
		t.Fatalf("bo sees %+v", r)
	}
	// Requesting twice is idempotent and does not notify twice.
	_, _ = s.Relate("ana", "bo", ActionRequest)
	if n, _ := s.Notifications("bo", 0); n.Unread != 1 || n.Items[0].Type != contracts.NotifyFriendRequest || n.Items[0].FromUserID != "ana" {
		t.Fatalf("bo inbox: %+v", n)
	}
	// Accepting is a request in the other direction.
	if r, _ := s.Relate("bo", "ana", ActionRequest); r.State != contracts.RelationFriend {
		t.Fatalf("mutual request must befriend: %+v", r)
	}
	if n, _ := s.Notifications("ana", 0); n.Unread != 1 || n.Items[0].Type != contracts.NotifyFriendAccepted {
		t.Fatalf("ana inbox: %+v", n)
	}
	if rel, _ := s.Relations("ana"); len(rel) != 1 || rel[0].UserID != "bo" || rel[0].State != contracts.RelationFriend {
		t.Fatalf("relations: %+v", rel)
	}
	// Unfriend clears both sides.
	if r, _ := s.Relate("bo", "ana", ActionRemove); r.State != contracts.RelationNone {
		t.Fatalf("remove: %+v", r)
	}
	if r, _ := s.Relation("ana", "bo"); r.State != contracts.RelationNone {
		t.Fatalf("ana still %+v", r)
	}
	// Decline and cancel.
	_, _ = s.Relate("ana", "bo", ActionRequest)
	if r, err := s.Relate("bo", "ana", ActionDecline); err != nil || r.State != contracts.RelationNone {
		t.Fatalf("decline: %+v %v", r, err)
	}
	if _, err := s.Relate("bo", "ana", ActionDecline); code(err) != "not-found" {
		t.Fatalf("decline without request: %v", err)
	}
	if _, err := s.Relate("bo", "ana", ActionAccept); code(err) != "not-found" {
		t.Fatalf("accept without request: %v", err)
	}
	_, _ = s.Relate("ana", "bo", ActionRequest)
	_, _ = s.Relate("ana", "bo", ActionRemove)
	if r, _ := s.Relation("bo", "ana"); r.State != contracts.RelationNone {
		t.Fatalf("cancel left %+v", r)
	}
	if _, err := s.Relate("ana", "ana", ActionRequest); code(err) != "invalid-message" {
		t.Fatalf("self: %v", err)
	}
	if _, err := s.Relate("ana", "bo", "poke"); code(err) != "invalid-message" {
		t.Fatalf("unknown action: %v", err)
	}
}

func TestRequestsCanBeClosed(t *testing.T) {
	s, _ := newService(t)
	nobody := contracts.RequestsNobody
	if _, err := s.PatchSettings("bo", SettingsPatch{AllowRequests: &nobody}); err != nil {
		t.Fatal(err)
	}
	if _, err := s.Relate("ana", "bo", ActionRequest); code(err) != "forbidden" {
		t.Fatalf("closed requests: %v", err)
	}
	// bo can still reach out.
	befriend(t, s, "bo", "ana")
}

func TestBlockIsSymmetricAndSilent(t *testing.T) {
	s, _ := newService(t)
	befriend(t, s, "ana", "bo")
	if r, _ := s.Relate("ana", "bo", ActionBlock); r.State != contracts.RelationBlocking {
		t.Fatalf("block: %+v", r)
	}
	r, _ := s.Relation("bo", "ana")
	if r.State != contracts.RelationNone || !r.BlockedBy {
		t.Fatalf("bo must lose the friendship and carry the hidden flag: %+v", r)
	}
	// bo's relation list never reveals the block.
	if rel, _ := s.Relations("bo"); len(rel) != 0 {
		t.Fatalf("bo relations: %+v", rel)
	}
	// bo's request reads as an unknown account; ana must unblock first.
	if _, err := s.Relate("bo", "ana", ActionRequest); code(err) != "not-found" {
		t.Fatalf("blocked request: %v", err)
	}
	if _, err := s.Relate("ana", "bo", ActionRequest); code(err) != "forbidden" {
		t.Fatalf("blocker request: %v", err)
	}
	// Even public content is hidden both ways.
	pub := contracts.VisibilityPublic
	_, _ = s.PatchSettings("ana", SettingsPatch{Activity: &pub, Ratings: &pub})
	_, _ = s.PatchSettings("bo", SettingsPatch{Activity: &pub, Ratings: &pub})
	for _, pair := range [][2]string{{"ana", "bo"}, {"bo", "ana"}} {
		if ok, _ := s.CanSee(pair[0], pair[1], SubjectProfile); ok {
			t.Fatalf("%s sees %s across a block", pair[0], pair[1])
		}
	}
	// Mutual block: one side lifting theirs keeps the other in force.
	_, _ = s.Relate("bo", "ana", ActionBlock)
	_, _ = s.Relate("ana", "bo", ActionUnblock)
	if r, _ := s.Relation("ana", "bo"); r.State != contracts.RelationNone || !r.BlockedBy {
		t.Fatalf("ana after unblock under bo's block: %+v", r)
	}
	_, _ = s.Relate("bo", "ana", ActionUnblock)
	if r, _ := s.Relation("ana", "bo"); r.State != contracts.RelationNone || r.BlockedBy {
		t.Fatalf("both unblocked: %+v", r)
	}
	befriend(t, s, "ana", "bo")
}

func TestPrivacyMatrix(t *testing.T) {
	s, _ := newService(t)
	befriend(t, s, "owner", "friend")
	levels := map[string]map[string]bool{
		contracts.VisibilityPublic:  {"owner": true, "friend": true, "stranger": true},
		contracts.VisibilityFriends: {"owner": true, "friend": true, "stranger": false},
		contracts.VisibilityPrivate: {"owner": true, "friend": false, "stranger": false},
	}
	for level, want := range levels {
		lv := level
		_, err := s.PatchSettings("owner", SettingsPatch{Profile: &lv, Activity: &lv, Ratings: &lv})
		if err != nil {
			t.Fatal(err)
		}
		for viewer, ok := range want {
			for _, subj := range []string{SubjectProfile, SubjectActivity, SubjectRatings} {
				got, err := s.CanSee(viewer, "owner", subj)
				if err != nil || got != ok {
					t.Errorf("%s/%s/%s = %v %v, want %v", level, viewer, subj, got, err, ok)
				}
			}
		}
	}
	bad := "everyone"
	if _, err := s.PatchSettings("owner", SettingsPatch{Profile: &bad}); code(err) != "invalid-message" {
		t.Fatalf("bad level: %v", err)
	}
	if _, err := s.CanSee("friend", "owner", "diary"); code(err) != "invalid-message" {
		t.Fatalf("bad subject: %v", err)
	}
}

func TestSettingsDefaultsAndFavorites(t *testing.T) {
	s, _ := newService(t)
	st, err := s.Settings("ana")
	if err != nil || st.Profile != contracts.VisibilityPublic || st.Activity != contracts.VisibilityFriends ||
		st.Ratings != contracts.VisibilityFriends || !st.Discoverable || st.AllowRequests != contracts.RequestsEveryone || st.Favorites == nil {
		t.Fatalf("defaults: %+v %v", st, err)
	}
	favs := []contracts.WorkRef{frieren, {Kind: "MANGA", Title: "frieren "}, {Kind: "comic", Title: "Saga"}}
	st, err = s.PatchSettings("ana", SettingsPatch{Favorites: &favs})
	if err != nil || len(st.Favorites) != 2 || st.Favorites[1].Kind != "comic" {
		t.Fatalf("favorites dedupe: %+v %v", st.Favorites, err)
	}
	// A patch without favorites keeps them.
	off := false
	st, _ = s.PatchSettings("ana", SettingsPatch{Discoverable: &off})
	if len(st.Favorites) != 2 || st.Discoverable {
		t.Fatalf("partial patch: %+v", st)
	}
	many := make([]contracts.WorkRef, 13)
	for i := range many {
		many[i] = contracts.WorkRef{Kind: "anime", Title: strings.Repeat("x", i+1)}
	}
	if _, err := s.PatchSettings("ana", SettingsPatch{Favorites: &many}); code(err) != "invalid-message" {
		t.Fatalf("13 favorites: %v", err)
	}
}

func TestProgressActivityIsDeduplicated(t *testing.T) {
	s, c := newService(t)
	rec := func(ep int, done, was bool) string {
		t.Helper()
		out, err := s.RecordProgress(RecordProgressInput{UserID: "ana", Work: contracts.WorkRef{Kind: "anime", Title: "Frieren", ItemID: "abc"}, Episode: ep, Completed: done, WasCompleted: was})
		if err != nil {
			t.Fatal(err)
		}
		return out.Recorded
	}
	if got := rec(1, false, false); got != contracts.ActivityProgress {
		t.Fatalf("first progress: %q", got)
	}
	c.advance(time.Minute)
	if got := rec(1, false, false); got != "" {
		t.Fatalf("repeat inside window: %q", got)
	}
	if got := rec(1, true, false); got != contracts.ActivityCompleted {
		t.Fatalf("completion: %q", got)
	}
	if got := rec(1, true, true); got != "" {
		t.Fatalf("already completed: %q", got)
	}
	c.advance(progressWindow)
	if got := rec(2, false, false); got != contracts.ActivityProgress {
		t.Fatalf("after window: %q", got)
	}
	rows, _ := s.UserActivity(UserActivityInput{ViewerID: "ana", OwnerID: "ana"})
	if len(rows) != 3 || rows[0].Episode != 2 || rows[1].Type != contracts.ActivityCompleted {
		t.Fatalf("rows: %+v", rows)
	}
}

func TestActivityIsCapped(t *testing.T) {
	s, c := newService(t)
	for i := 0; i < maxActivityRows+20; i++ {
		c.advance(time.Second)
		if _, err := s.RecordProgress(RecordProgressInput{UserID: "ana", Work: contracts.WorkRef{Kind: "comic", Title: "Saga"}, Completed: true}); err != nil {
			t.Fatal(err)
		}
	}
	rows, _ := s.UserActivity(UserActivityInput{ViewerID: "ana", OwnerID: "ana", Limit: maxPageLimit})
	if len(rows) != maxPageLimit {
		t.Fatalf("page: %d", len(rows))
	}
	n := 0
	before := int64(0)
	for {
		page, _ := s.UserActivity(UserActivityInput{ViewerID: "ana", OwnerID: "ana", Before: before, Limit: maxPageLimit})
		if len(page) == 0 {
			break
		}
		n += len(page)
		before = page[len(page)-1].At
	}
	if n != maxActivityRows {
		t.Fatalf("kept %d rows, want %d", n, maxActivityRows)
	}
}

func TestFeedRespectsPrivacy(t *testing.T) {
	s, c := newService(t)
	befriend(t, s, "me", "open")
	befriend(t, s, "me", "quiet")
	befriend(t, s, "me", "shy")
	priv := contracts.VisibilityPrivate
	yes := true
	_, _ = s.PatchSettings("quiet", SettingsPatch{Activity: &priv})
	_, _ = s.PatchSettings("shy", SettingsPatch{HideProgress: &yes, Ratings: &priv})
	for _, u := range []string{"open", "quiet", "shy", "stranger"} {
		c.advance(time.Second)
		_, _ = s.RecordProgress(RecordProgressInput{UserID: u, Work: frieren})
		c.advance(time.Second)
		_, _ = s.RecordProgress(RecordProgressInput{UserID: u, Work: frieren, Completed: true})
		c.advance(time.Second)
		if _, err := s.PutRating(PutRatingInput{UserID: u, Work: frieren, Score: 9}); err != nil {
			t.Fatal(err)
		}
	}
	feed, err := s.Feed(FeedInput{UserID: "me"})
	if err != nil {
		t.Fatal(err)
	}
	got := map[string][]string{}
	for _, a := range feed {
		got[a.UserID] = append(got[a.UserID], a.Type)
	}
	if strings.Join(got["open"], ",") != "rated,completed,progress" {
		t.Errorf("open: %v", got["open"])
	}
	if len(got["quiet"]) != 0 || len(got["stranger"]) != 0 || len(got["me"]) != 0 {
		t.Errorf("leak: %v", got)
	}
	if strings.Join(got["shy"], ",") != "completed" {
		t.Errorf("shy (progress hidden, ratings private): %v", got["shy"])
	}
	for i := 1; i < len(feed); i++ {
		if feed[i].At > feed[i-1].At {
			t.Fatal("feed must be newest first")
		}
	}
	// Paging by time.
	page, _ := s.Feed(FeedInput{UserID: "me", Limit: 2})
	rest, _ := s.Feed(FeedInput{UserID: "me", Before: page[1].At})
	if len(page) != 2 || len(rest) != len(feed)-2 {
		t.Fatalf("paging: %d + %d of %d", len(page), len(rest), len(feed))
	}
}

func TestRatingsAndReviews(t *testing.T) {
	s, c := newService(t)
	befriend(t, s, "ana", "bo")
	r, err := s.PutRating(PutRatingInput{UserID: "ana", Work: frieren, Score: 8, Review: "  quiet\nand kind  "})
	if err != nil || r.Score != 8 || r.Review != "quiet\nand kind" {
		t.Fatalf("put: %+v %v", r, err)
	}
	created := r.CreatedAt
	c.advance(time.Hour)
	r, _ = s.PutRating(PutRatingInput{UserID: "ana", Work: contracts.WorkRef{Kind: "manga", Title: "FRIEREN"}, Score: 10})
	if r.CreatedAt != created || r.UpdatedAt == created {
		t.Fatalf("update must keep created_at: %+v", r)
	}
	_, _ = s.PutRating(PutRatingInput{UserID: "bo", Work: frieren, Score: 6})
	_, _ = s.PutRating(PutRatingInput{UserID: "cy", Work: frieren, Score: 1}) // stranger, friends-only
	v, err := s.Work("ana", frieren)
	if err != nil || v.Mine == nil || v.Mine.Score != 10 || len(v.Ratings) != 1 || v.Ratings[0].UserID != "bo" {
		t.Fatalf("view: %+v %v", v, err)
	}
	if v.Scored != 2 || v.Average != 8 {
		t.Fatalf("average must count only visible scores: %v over %d", v.Average, v.Scored)
	}
	// The anime of the same title is another work.
	if v, _ := s.Work("ana", contracts.WorkRef{Kind: "anime", Title: "Frieren"}); v.Mine != nil || len(v.Ratings) != 0 {
		t.Fatalf("anime view leaked manga ratings: %+v", v)
	}
	if list, _ := s.UserRatings("bo", "ana"); len(list) != 1 || list[0].Score != 10 {
		t.Fatalf("bo reads ana's ratings: %+v", list)
	}
	if list, _ := s.UserRatings("cy", "ana"); len(list) != 0 {
		t.Fatalf("stranger reads friends-only ratings: %+v", list)
	}
	for _, bad := range []PutRatingInput{
		{UserID: "ana", Work: frieren, Score: 11},
		{UserID: "ana", Work: frieren, Score: -1},
		{UserID: "ana", Work: frieren},
		{UserID: "ana", Work: frieren, Score: 5, Review: strings.Repeat("x", 2001)},
	} {
		if _, err := s.PutRating(bad); code(err) != "invalid-message" {
			t.Errorf("%+v: %v", bad, err)
		}
	}
	// A review without a score is allowed.
	if _, err := s.PutRating(PutRatingInput{UserID: "bo", Work: contracts.WorkRef{Kind: "comic", Title: "Saga"}, Review: "yes"}); err != nil {
		t.Fatal(err)
	}
	if ok, _ := s.DeleteRating("ana", frieren); !ok {
		t.Fatal("delete must report the rating existed")
	}
	if v, _ := s.Work("ana", frieren); v.Mine != nil {
		t.Fatal("rating survived delete")
	}
	if list, _ := s.UserRatings("ana", "ana"); len(list) != 0 {
		t.Fatalf("by-user index survived delete: %+v", list)
	}
}

func TestCommentsRepliesAndModeration(t *testing.T) {
	s, _ := newService(t)
	top, err := s.AddComment(AddCommentInput{UserID: "ana", Work: frieren, Body: "chapter 60 tho"})
	if err != nil {
		t.Fatal(err)
	}
	reply, err := s.AddComment(AddCommentInput{UserID: "bo", Work: frieren, Body: "yes", ReplyTo: top.ID})
	if err != nil || reply.ReplyTo != top.ID {
		t.Fatalf("reply: %+v %v", reply, err)
	}
	if n, _ := s.Notifications("ana", 0); n.Unread != 1 || n.Items[0].Type != contracts.NotifyReply || n.Items[0].CommentID != reply.ID {
		t.Fatalf("reply notification: %+v", n)
	}
	// Replying to yourself notifies nobody.
	_, _ = s.AddComment(AddCommentInput{UserID: "ana", Work: frieren, Body: "and 61", ReplyTo: top.ID})
	if n, _ := s.Notifications("ana", 0); n.Unread != 1 {
		t.Fatalf("self reply notified: %+v", n)
	}
	// A reply must target the same work.
	if _, err := s.AddComment(AddCommentInput{UserID: "bo", Work: contracts.WorkRef{Kind: "anime", Title: "Frieren"}, Body: "x", ReplyTo: top.ID}); code(err) != "not-found" {
		t.Fatalf("cross-work reply: %v", err)
	}
	if _, err := s.AddComment(AddCommentInput{UserID: "bo", Work: frieren, Body: "   "}); code(err) != "invalid-message" {
		t.Fatalf("empty: %v", err)
	}
	v, _ := s.Work("cy", frieren)
	if len(v.Comments) != 3 || v.Comments[0].ID != top.ID {
		t.Fatalf("comments in order: %+v", v.Comments)
	}
	// Blocked authors vanish from the viewer's thread.
	_, _ = s.Relate("cy", "bo", ActionBlock)
	if v, _ := s.Work("cy", frieren); len(v.Comments) != 2 {
		t.Fatalf("blocked author still shown: %+v", v.Comments)
	}
	if _, err := s.DeleteComment(DeleteCommentInput{UserID: "cy", ID: top.ID}); code(err) != "forbidden" {
		t.Fatalf("stranger delete: %v", err)
	}
	if _, err := s.DeleteComment(DeleteCommentInput{UserID: "cy", ID: top.ID, Moderator: true}); err != nil {
		t.Fatalf("moderator delete: %v", err)
	}
	if _, err := s.DeleteComment(DeleteCommentInput{UserID: "ana", ID: top.ID}); code(err) != "not-found" {
		t.Fatalf("double delete: %v", err)
	}
}

func TestCollectionsVisibilityAndSharing(t *testing.T) {
	s, _ := newService(t)
	befriend(t, s, "ana", "bo")
	col, err := s.CreateCollection(CreateCollectionInput{UserID: "ana", Name: "  Cozy   reads ", Description: "for rain"})
	if err != nil || col.Name != "Cozy reads" || col.Visibility != contracts.VisibilityPrivate {
		t.Fatalf("create: %+v %v", col, err)
	}
	for _, w := range []contracts.WorkRef{frieren, {Kind: "comic", Title: "Saga"}, {Kind: "movie", Title: "Perfect Blue"}} {
		if _, err := s.AddCollectionItem(AddCollectionItemInput{UserID: "ana", ID: col.ID, Work: w}); err != nil {
			t.Fatal(err)
		}
	}
	col, _ = s.AddCollectionItem(AddCollectionItemInput{UserID: "ana", ID: col.ID, Work: contracts.WorkRef{Kind: "manga", Title: "frieren"}, Note: "start here"})
	if len(col.Items) != 3 || col.Items[0].Note != "start here" {
		t.Fatalf("re-add must update the note in place: %+v", col.Items)
	}
	// Private: bo cannot open it, and it reads as unknown.
	if _, err := s.Collection("bo", col.ID); code(err) != "not-found" {
		t.Fatalf("private open: %v", err)
	}
	if _, err := s.AddCollectionItem(AddCollectionItemInput{UserID: "bo", ID: col.ID, Work: frieren}); code(err) != "not-found" {
		t.Fatalf("stranger edit of hidden collection: %v", err)
	}
	// Sharing targets friends only.
	if _, err := s.ShareCollection(ShareCollectionInput{UserID: "ana", ID: col.ID, To: []string{"cy"}}); code(err) != "forbidden" {
		t.Fatalf("share with stranger: %v", err)
	}
	out, err := s.ShareCollection(ShareCollectionInput{UserID: "ana", ID: col.ID, To: []string{"bo", "bo", "ana"}, Message: "rainy day"})
	if err != nil || len(out.Sent) != 1 {
		t.Fatalf("share: %+v %v", out, err)
	}
	got, err := s.Collection("bo", col.ID)
	if err != nil || len(got.SharedWith) != 0 {
		t.Fatalf("bo opens shared collection without seeing the share list: %+v %v", got, err)
	}
	if _, err := s.AddCollectionItem(AddCollectionItemInput{UserID: "bo", ID: col.ID, Work: frieren}); code(err) != "forbidden" {
		t.Fatalf("shared is read-only: %v", err)
	}
	if mine, _ := s.Collections("bo", ""); len(mine) != 1 || mine[0].ID != col.ID {
		t.Fatalf("shared with me: %+v", mine)
	}
	if n, _ := s.Notifications("bo", 0); n.Items[0].Type != contracts.NotifyCollectionShared || n.Items[0].Message != "rainy day" {
		t.Fatalf("share notification: %+v", n.Items[0])
	}
	// Unfriending revokes the explicit share without editing it.
	_, _ = s.Relate("bo", "ana", ActionRemove)
	if _, err := s.Collection("bo", col.ID); code(err) != "not-found" {
		t.Fatalf("ex-friend still opens: %v", err)
	}
	// Public collections show on the profile; the profile gate applies.
	pub := contracts.VisibilityPublic
	_, _ = s.UpdateCollection(UpdateCollectionInput{UserID: "ana", ID: col.ID, Visibility: &pub})
	if list, _ := s.Collections("cy", "ana"); len(list) != 1 {
		t.Fatalf("public collection on profile: %+v", list)
	}
	priv := contracts.VisibilityPrivate
	_, _ = s.PatchSettings("ana", SettingsPatch{Profile: &priv})
	if list, _ := s.Collections("cy", "ana"); len(list) != 0 {
		t.Fatalf("private profile still lists collections: %+v", list)
	}
	col, _ = s.RemoveCollectionItem(RemoveCollectionItemInput{UserID: "ana", ID: col.ID, Work: frieren})
	if len(col.Items) != 2 {
		t.Fatalf("remove: %+v", col.Items)
	}
	if _, err := s.DeleteCollection("cy", col.ID); code(err) != "not-found" {
		t.Fatalf("stranger delete: %v", err)
	}
	if ok, err := s.DeleteCollection("ana", col.ID); !ok || err != nil {
		t.Fatalf("delete: %v", err)
	}
	if _, err := s.CreateCollection(CreateCollectionInput{UserID: "ana", Name: " "}); code(err) != "invalid-message" {
		t.Fatalf("empty name: %v", err)
	}
}

func TestShareWorkAndNotifications(t *testing.T) {
	s, _ := newService(t)
	befriend(t, s, "ana", "bo")
	befriend(t, s, "ana", "cy")
	if _, err := s.ShareWork(ShareWorkInput{UserID: "ana", To: []string{"dee"}, Work: frieren}); code(err) != "forbidden" {
		t.Fatalf("stranger share: %v", err)
	}
	if _, err := s.ShareWork(ShareWorkInput{UserID: "ana", Work: frieren}); code(err) != "invalid-message" {
		t.Fatalf("no recipients: %v", err)
	}
	out, err := s.ShareWork(ShareWorkInput{UserID: "ana", To: []string{"bo", "cy"}, Work: frieren, Message: "read this"})
	if err != nil || len(out.Sent) != 2 {
		t.Fatalf("share: %+v %v", out, err)
	}
	n, _ := s.Notifications("bo", 0)
	share := n.Items[0]
	if share.Type != contracts.NotifyShare || share.Work == nil || share.Work.Title != "Frieren" || share.Message != "read this" {
		t.Fatalf("share row: %+v", share)
	}
	before := n.Unread
	if changed, _ := s.MarkRead(MarkReadInput{UserID: "bo", IDs: []string{share.ID}}); changed != 1 {
		t.Fatalf("mark one: %d", changed)
	}
	if n, _ := s.Notifications("bo", 0); n.Unread != before-1 {
		t.Fatalf("unread after mark: %d", n.Unread)
	}
	_, _ = s.MarkRead(MarkReadInput{UserID: "bo", All: true})
	if n, _ := s.Notifications("bo", 0); n.Unread != 0 {
		t.Fatalf("unread after all: %d", n.Unread)
	}
	// Blocking the sender hides old rows too.
	_, _ = s.Relate("cy", "ana", ActionBlock)
	if n, _ := s.Notifications("cy", 0); len(n.Items) != 0 {
		t.Fatalf("blocked sender rows: %+v", n.Items)
	}
}

func TestNotificationsAreCapped(t *testing.T) {
	s, c := newService(t)
	befriend(t, s, "ana", "bo")
	for i := 0; i < maxNotifications+10; i++ {
		c.advance(time.Second)
		if _, err := s.ShareWork(ShareWorkInput{UserID: "ana", To: []string{"bo"}, Work: frieren}); err != nil {
			t.Fatal(err)
		}
	}
	n, _ := s.Notifications("bo", maxPageLimit)
	if n.Unread != maxNotifications {
		t.Fatalf("kept %d unread, want %d", n.Unread, maxNotifications)
	}
}

func TestInvokeRejectsWrongInputs(t *testing.T) {
	s, _ := newService(t)
	for _, cap := range s.Capabilities() {
		if _, err := s.Invoke(cap, struct{}{}); code(err) != "invalid-message" {
			t.Errorf("%s: %v", cap, err)
		}
	}
	if _, err := s.Invoke(contracts.CapSocialGraph, GetSettingsInput{}); code(err) != "invalid-message" {
		t.Fatalf("empty user: %v", err)
	}
	out, err := s.Invoke(contracts.CapSocialGraph, RelateInput{UserID: "a", OtherID: "b", Action: ActionRequest})
	if err != nil || out.(RelationOutput).State != contracts.RelationOutgoing {
		t.Fatalf("invoke relate: %+v %v", out, err)
	}
}

func TestClosedDBDegrades(t *testing.T) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	s, err := New(db)
	if err != nil {
		t.Fatal(err)
	}
	db.Close()
	if _, err := s.Relate("a", "b", ActionRequest); err == nil {
		t.Fatal("write on a closed db must error")
	}
	if _, err := s.PutRating(PutRatingInput{UserID: "a", Work: frieren, Score: 3}); err == nil {
		t.Fatal("rating on a closed db must error")
	}
}
