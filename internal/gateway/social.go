// Social routes (docs/slices/social.md). The gateway owns identity —
// who is signed in, which accounts exist, which catalog item a work
// names — and the lain-social-bolt provider behind the lain.social.*
// capabilities owns every relationship and privacy rule. Handlers
// resolve, call, and attach the public summaries of the accounts a
// response mentions; they never decide visibility themselves.
package gateway

import (
	"net/http"
	"strconv"
	"strings"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/social"
)

func (s *Server) routesSocial() {
	m := s.mux
	m.HandleFunc("GET /api/social/me/settings", s.requireAuth(s.handleSocialSettingsGet))
	m.HandleFunc("PATCH /api/social/me/settings", s.requireAuth(s.handleSocialSettingsPatch))

	m.HandleFunc("GET /api/social/users", s.requireAuth(s.handleSocialUserSearch))
	m.HandleFunc("GET /api/social/users/{id}", s.requireAuth(s.handleSocialProfile))
	m.HandleFunc("GET /api/social/users/{id}/activity", s.requireAuth(s.handleSocialUserActivity))
	m.HandleFunc("GET /api/social/users/{id}/ratings", s.requireAuth(s.handleSocialUserRatings))
	m.HandleFunc("GET /api/social/users/{id}/collections", s.requireAuth(s.handleSocialUserCollections))

	m.HandleFunc("GET /api/social/friends", s.requireAuth(s.handleSocialFriends))
	m.HandleFunc("POST /api/social/friends/{id}", s.requireAuth(s.relateHandler(social.ActionRequest)))
	m.HandleFunc("POST /api/social/friends/{id}/decline", s.requireAuth(s.relateHandler(social.ActionDecline)))
	m.HandleFunc("DELETE /api/social/friends/{id}", s.requireAuth(s.relateHandler(social.ActionRemove)))
	m.HandleFunc("POST /api/social/blocks/{id}", s.requireAuth(s.relateHandler(social.ActionBlock)))
	m.HandleFunc("DELETE /api/social/blocks/{id}", s.requireAuth(s.relateHandler(social.ActionUnblock)))

	m.HandleFunc("GET /api/social/feed", s.requireAuth(s.handleSocialFeed))
	m.HandleFunc("GET /api/social/notifications", s.requireAuth(s.handleSocialNotifications))
	m.HandleFunc("POST /api/social/notifications/read", s.requireAuth(s.handleSocialNotificationsRead))

	m.HandleFunc("GET /api/social/work", s.requireAuth(s.handleSocialWork))
	m.HandleFunc("PUT /api/social/work/rating", s.requireAuth(s.handleSocialRatingPut))
	m.HandleFunc("DELETE /api/social/work/rating", s.requireAuth(s.handleSocialRatingDelete))
	m.HandleFunc("POST /api/social/work/comments", s.requireAuth(s.handleSocialCommentAdd))
	m.HandleFunc("DELETE /api/social/comments/{id}", s.requireAuth(s.handleSocialCommentDelete))
	m.HandleFunc("POST /api/social/share", s.requireAuth(s.handleSocialShare))

	m.HandleFunc("GET /api/social/collections", s.requireAuth(s.handleSocialCollections))
	m.HandleFunc("POST /api/social/collections", s.requireAuth(s.handleSocialCollectionCreate))
	m.HandleFunc("GET /api/social/collections/{id}", s.requireAuth(s.handleSocialCollectionGet))
	m.HandleFunc("PATCH /api/social/collections/{id}", s.requireAuth(s.handleSocialCollectionPatch))
	m.HandleFunc("DELETE /api/social/collections/{id}", s.requireAuth(s.handleSocialCollectionDelete))
	m.HandleFunc("POST /api/social/collections/{id}/items", s.requireAuth(s.handleSocialCollectionAdd))
	m.HandleFunc("DELETE /api/social/collections/{id}/items", s.requireAuth(s.handleSocialCollectionRemove))
	m.HandleFunc("POST /api/social/collections/{id}/share", s.requireAuth(s.handleSocialCollectionShare))
	m.HandleFunc("DELETE /api/social/collections/{id}/share/{user}", s.requireAuth(s.handleSocialCollectionUnshare))
}

// userSummary is the public face of an account in social responses:
// what any signed-in account already sees (D-086).
type userSummary struct {
	ID          string      `json:"id"`
	Username    string      `json:"username"`
	DisplayName string      `json:"display_name,omitempty"`
	Avatar      auth.Avatar `json:"avatar"`
}

func summaryOf(u auth.User) userSummary {
	return userSummary{ID: u.ID, Username: u.Username, DisplayName: u.Profile.DisplayName, Avatar: u.Profile.Avatar}
}

// activeUser returns an existing, enabled account.
func (s *Server) activeUser(id string) (auth.User, bool) {
	u, ok := s.auth.Get(id)
	if !ok || u.Disabled {
		return auth.User{}, false
	}
	return u, true
}

// summaries resolves account ids into the users map a response carries.
// Unknown and disabled accounts are left out; clients render them as
// a removed account.
func (s *Server) summaries(ids ...string) map[string]userSummary {
	out := map[string]userSummary{}
	for _, id := range ids {
		if _, done := out[id]; done || id == "" {
			continue
		}
		if u, ok := s.activeUser(id); ok {
			out[id] = summaryOf(u)
		}
	}
	return out
}

// socialCall routes one call through the registry.
func (s *Server) socialCall(cap string, in any) (any, error) {
	out, _, err := s.reg.CallOne(cap, in)
	return out, err
}

// writeSocialErr maps typed provider codes onto HTTP and keeps the code
// in the body so clients can branch without parsing messages.
func (s *Server) writeSocialErr(w http.ResponseWriter, err error) {
	ce, ok := err.(*core.Error)
	if !ok {
		s.logger().Warn("social call failed", "err", err.Error())
		writeErr(w, http.StatusServiceUnavailable, "social service unavailable")
		return
	}
	code := http.StatusServiceUnavailable
	switch ce.Code {
	case "invalid-message":
		code = http.StatusBadRequest
	case "not-found":
		code = http.StatusNotFound
	case "forbidden":
		code = http.StatusForbidden
	default:
		s.logger().Warn("social call failed", "code", ce.Code, "err", ce.Msg)
	}
	writeJSON(w, code, map[string]string{"error": ce.Msg, "code": ce.Code})
}

// resolveWork fills a work's kind and title from the catalog when the
// client names an item: the catalog is authoritative for files it
// owns. A work named only by kind and title passes through (list
// entries and other works with no local file).
func (s *Server) resolveWork(w contracts.WorkRef) (contracts.WorkRef, error) {
	if w.ItemID != "" {
		it, ok := s.catGet(w.ItemID)
		if !ok {
			if w.Kind == "" || w.Title == "" {
				return w, &core.Error{Code: "not-found", Msg: "unknown item"}
			}
			w.ItemID = ""
		} else {
			w.Kind, w.Title = it.Kind, it.Title
		}
	}
	return social.NormalizeWork(w)
}

func workFromQuery(r *http.Request) contracts.WorkRef {
	q := r.URL.Query()
	return contracts.WorkRef{ItemID: q.Get("item"), Kind: q.Get("kind"), Title: q.Get("title")}
}

// targetUser resolves the {id} path account for an action. Unknown,
// disabled and blocked-by accounts all read the same: not found.
func (s *Server) targetUser(w http.ResponseWriter, r *http.Request, viewer string) (auth.User, social.RelationOutput, bool) {
	id := r.PathValue("id")
	u, ok := s.activeUser(id)
	if !ok {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "unknown user", "code": "not-found"})
		return auth.User{}, social.RelationOutput{}, false
	}
	if id == viewer {
		return u, social.RelationOutput{State: contracts.RelationNone}, true
	}
	out, err := s.socialCall(contracts.CapSocialGraph, social.RelationInput{UserID: viewer, OtherID: id})
	if err != nil {
		s.writeSocialErr(w, err)
		return auth.User{}, social.RelationOutput{}, false
	}
	rel := out.(social.RelationOutput)
	if rel.BlockedBy {
		writeJSON(w, http.StatusNotFound, map[string]string{"error": "unknown user", "code": "not-found"})
		return auth.User{}, social.RelationOutput{}, false
	}
	return u, rel, true
}

func queryInt64(r *http.Request, key string) int64 {
	n, _ := strconv.ParseInt(r.URL.Query().Get(key), 10, 64)
	return n
}

// ---- settings ----

func (s *Server) handleSocialSettingsGet(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, err := s.socialCall(contracts.CapSocialGraph, social.GetSettingsInput{UserID: v.UserID})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, out)
}

func (s *Server) handleSocialSettingsPatch(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var p social.SettingsPatch
	if !s.decode(w, r, &p) {
		return
	}
	if p.Favorites != nil {
		favs := make([]contracts.WorkRef, 0, len(*p.Favorites))
		for _, f := range *p.Favorites {
			rw, err := s.resolveWork(f)
			if err != nil {
				s.writeSocialErr(w, err)
				return
			}
			favs = append(favs, rw)
		}
		p.Favorites = &favs
	}
	out, err := s.socialCall(contracts.CapSocialGraph, social.PatchSettingsInput{UserID: v.UserID, Patch: p})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, out)
}

// ---- people ----

type userWithRelation struct {
	User     userSummary `json:"user"`
	Relation string      `json:"relation"`
}

// handleSocialUserSearch finds accounts by username or display name.
// Non-discoverable accounts appear only to accounts already related to
// them; blocks in either direction hide the account.
func (s *Server) handleSocialUserSearch(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	q := strings.ToLower(strings.TrimSpace(r.URL.Query().Get("q")))
	out := []userWithRelation{}
	for _, u := range s.auth.List() {
		if u.ID == v.UserID || u.Disabled {
			continue
		}
		if q != "" && !strings.Contains(strings.ToLower(u.Username), q) && !strings.Contains(strings.ToLower(u.Profile.DisplayName), q) {
			continue
		}
		relOut, err := s.socialCall(contracts.CapSocialGraph, social.RelationInput{UserID: v.UserID, OtherID: u.ID})
		if err != nil {
			s.writeSocialErr(w, err)
			return
		}
		rel := relOut.(social.RelationOutput)
		if rel.BlockedBy || rel.State == contracts.RelationBlocking {
			continue
		}
		if rel.State == contracts.RelationNone {
			stOut, err := s.socialCall(contracts.CapSocialGraph, social.GetSettingsInput{UserID: u.ID})
			if err != nil {
				s.writeSocialErr(w, err)
				return
			}
			if !stOut.(contracts.SocialSettings).Discoverable {
				continue
			}
		}
		out = append(out, userWithRelation{User: summaryOf(u), Relation: rel.State})
		if len(out) == 50 {
			break
		}
	}
	writeJSON(w, http.StatusOK, map[string]any{"users": out})
}

type profileVisibility struct {
	Profile  bool `json:"profile"`
	Activity bool `json:"activity"`
	Ratings  bool `json:"ratings"`
}

// handleSocialProfile is another account's profile as the viewer may
// see it: identity always, bio and favorites behind the profile level.
func (s *Server) handleSocialProfile(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	u, rel, ok := s.targetUser(w, r, v.UserID)
	if !ok {
		return
	}
	var vis profileVisibility
	for subject, dst := range map[string]*bool{social.SubjectProfile: &vis.Profile, social.SubjectActivity: &vis.Activity, social.SubjectRatings: &vis.Ratings} {
		out, err := s.socialCall(contracts.CapSocialGraph, social.CanSeeInput{ViewerID: v.UserID, OwnerID: u.ID, Subject: subject})
		if err != nil {
			s.writeSocialErr(w, err)
			return
		}
		*dst = out.(bool)
	}
	resp := map[string]any{
		"user":      summaryOf(u),
		"relation":  rel.State,
		"self":      u.ID == v.UserID,
		"visible":   vis,
		"favorites": []contracts.WorkRef{},
	}
	if vis.Profile {
		stOut, err := s.socialCall(contracts.CapSocialGraph, social.GetSettingsInput{UserID: u.ID})
		if err != nil {
			s.writeSocialErr(w, err)
			return
		}
		resp["bio"] = u.Profile.Bio
		resp["favorites"] = stOut.(contracts.SocialSettings).Favorites
	}
	writeJSON(w, http.StatusOK, resp)
}

func (s *Server) handleSocialUserActivity(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	u, _, ok := s.targetUser(w, r, v.UserID)
	if !ok {
		return
	}
	out, err := s.socialCall(contracts.CapSocialActivity, social.UserActivityInput{ViewerID: v.UserID, OwnerID: u.ID, Before: queryInt64(r, "before"), Limit: atoiQuery(r, "limit")})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": out, "users": s.summaries(u.ID)})
}

func (s *Server) handleSocialUserRatings(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	u, _, ok := s.targetUser(w, r, v.UserID)
	if !ok {
		return
	}
	out, err := s.socialCall(contracts.CapSocialReviews, social.UserRatingsInput{ViewerID: v.UserID, OwnerID: u.ID})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"ratings": out})
}

func (s *Server) handleSocialUserCollections(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	u, _, ok := s.targetUser(w, r, v.UserID)
	if !ok {
		return
	}
	out, err := s.socialCall(contracts.CapSocialCollections, social.ListCollectionsInput{ViewerID: v.UserID, OwnerID: u.ID})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"collections": out, "users": s.summaries(u.ID)})
}

type relationRow struct {
	User  userSummary `json:"user"`
	Since int64       `json:"since"`
}

func (s *Server) handleSocialFriends(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, err := s.socialCall(contracts.CapSocialGraph, social.ListRelationsInput{UserID: v.UserID})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	groups := map[string][]relationRow{
		contracts.RelationFriend: {}, contracts.RelationIncoming: {}, contracts.RelationOutgoing: {}, contracts.RelationBlocking: {},
	}
	for _, rel := range out.([]contracts.Relationship) {
		u, ok := s.auth.Get(rel.UserID)
		if !ok || (u.Disabled && rel.State != contracts.RelationBlocking) {
			continue
		}
		groups[rel.State] = append(groups[rel.State], relationRow{User: summaryOf(u), Since: rel.UpdatedAt})
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"friends":  groups[contracts.RelationFriend],
		"incoming": groups[contracts.RelationIncoming],
		"outgoing": groups[contracts.RelationOutgoing],
		"blocked":  groups[contracts.RelationBlocking],
	})
}

func (s *Server) relateHandler(action string) func(http.ResponseWriter, *http.Request, auth.Verified) {
	return func(w http.ResponseWriter, r *http.Request, v auth.Verified) {
		// Unblocking must still reach an account that blocked you back,
		// so only the existence check runs before the provider decides.
		u, ok := s.auth.Get(r.PathValue("id"))
		if !ok || (u.Disabled && action != social.ActionUnblock && action != social.ActionRemove) {
			writeJSON(w, http.StatusNotFound, map[string]string{"error": "unknown user", "code": "not-found"})
			return
		}
		out, err := s.socialCall(contracts.CapSocialGraph, social.RelateInput{UserID: v.UserID, OtherID: u.ID, Action: action})
		if err != nil {
			s.writeSocialErr(w, err)
			return
		}
		writeJSON(w, http.StatusOK, map[string]any{"user": summaryOf(u), "relation": out.(social.RelationOutput).State})
	}
}

// ---- feed and notifications ----

func (s *Server) handleSocialFeed(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, err := s.socialCall(contracts.CapSocialActivity, social.FeedInput{UserID: v.UserID, Before: queryInt64(r, "before"), Limit: atoiQuery(r, "limit")})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	rows := out.([]contracts.Activity)
	ids := make([]string, 0, len(rows))
	for _, a := range rows {
		ids = append(ids, a.UserID)
	}
	users := s.summaries(ids...)
	items := make([]contracts.Activity, 0, len(rows))
	for _, a := range rows {
		if _, ok := users[a.UserID]; ok {
			items = append(items, a)
		}
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": items, "users": users})
}

func (s *Server) handleSocialNotifications(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, err := s.socialCall(contracts.CapSocialActivity, social.NotificationsInput{UserID: v.UserID, Limit: atoiQuery(r, "limit")})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	n := out.(social.NotificationsOutput)
	ids := make([]string, 0, len(n.Items))
	for _, it := range n.Items {
		ids = append(ids, it.FromUserID)
	}
	writeJSON(w, http.StatusOK, map[string]any{"items": n.Items, "unread": n.Unread, "users": s.summaries(ids...)})
}

func (s *Server) handleSocialNotificationsRead(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		IDs []string `json:"ids"`
		All bool     `json:"all"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	out, err := s.socialCall(contracts.CapSocialActivity, social.MarkReadInput{UserID: v.UserID, IDs: in.IDs, All: in.All})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"changed": out})
}

// recordSocialProgress reports an accepted progress write to the
// activity log. It is best-effort: social bookkeeping never fails a
// progress write, but a failure is logged.
func (s *Server) recordSocialProgress(userID string, prev, next contracts.Progress) {
	it, ok := s.catGet(next.ItemID)
	if !ok {
		return
	}
	in := social.RecordProgressInput{
		UserID:       userID,
		Work:         contracts.WorkRef{Kind: it.Kind, Title: it.Title, ItemID: it.ID},
		Season:       it.Season,
		Episode:      it.Episode,
		Completed:    next.Completed,
		WasCompleted: prev.Completed,
	}
	if _, err := s.socialCall(contracts.CapSocialActivity, in); err != nil {
		s.logger().Warn("social activity not recorded", "user", userID, "item", next.ItemID, "err", err.Error())
	}
}

// ---- works: ratings, reviews, comments, shares ----

func (s *Server) handleSocialWork(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	work, err := s.resolveWork(workFromQuery(r))
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	out, err := s.socialCall(contracts.CapSocialReviews, social.WorkInput{ViewerID: v.UserID, Work: work})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	view := out.(social.WorkView)
	ids := []string{v.UserID}
	for _, rt := range view.Ratings {
		ids = append(ids, rt.UserID)
	}
	for _, c := range view.Comments {
		ids = append(ids, c.UserID)
	}
	writeJSON(w, http.StatusOK, map[string]any{
		"work": view.Work, "mine": view.Mine, "ratings": view.Ratings, "average": view.Average,
		"scored": view.Scored, "comments": view.Comments, "users": s.summaries(ids...),
	})
}

func (s *Server) handleSocialRatingPut(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		Work    contracts.WorkRef `json:"work"`
		Score   int               `json:"score"`
		Review  string            `json:"review"`
		Spoiler bool              `json:"spoiler"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	work, err := s.resolveWork(in.Work)
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	out, err := s.socialCall(contracts.CapSocialReviews, social.PutRatingInput{UserID: v.UserID, Work: work, Score: in.Score, Review: in.Review, Spoiler: in.Spoiler})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, out)
}

func (s *Server) handleSocialRatingDelete(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	work, err := s.resolveWork(workFromQuery(r))
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	out, err := s.socialCall(contracts.CapSocialReviews, social.DeleteRatingInput{UserID: v.UserID, Work: work})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": out})
}

func (s *Server) handleSocialCommentAdd(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		Work    contracts.WorkRef `json:"work"`
		Body    string            `json:"body"`
		ReplyTo string            `json:"reply_to"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	work, err := s.resolveWork(in.Work)
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	out, err := s.socialCall(contracts.CapSocialReviews, social.AddCommentInput{UserID: v.UserID, Work: work, Body: in.Body, ReplyTo: in.ReplyTo})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, out)
}

func (s *Server) handleSocialCommentDelete(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	_, err := s.socialCall(contracts.CapSocialReviews, social.DeleteCommentInput{UserID: v.UserID, ID: r.PathValue("id"), Moderator: v.Role == auth.RoleAdmin})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

func (s *Server) handleSocialShare(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		To      []string          `json:"to"`
		Work    contracts.WorkRef `json:"work"`
		Message string            `json:"message"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	work, err := s.resolveWork(in.Work)
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	out, err := s.socialCall(contracts.CapSocialCollections, social.ShareWorkInput{UserID: v.UserID, To: in.To, Work: work, Message: in.Message})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, out)
}

// ---- collections ----

func (s *Server) writeCollection(w http.ResponseWriter, code int, out any, err error) {
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	c := out.(contracts.Collection)
	writeJSON(w, code, map[string]any{"collection": c, "users": s.summaries(append([]string{c.OwnerID}, c.SharedWith...)...)})
}

func (s *Server) handleSocialCollections(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, err := s.socialCall(contracts.CapSocialCollections, social.ListCollectionsInput{ViewerID: v.UserID})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	cols := out.([]contracts.Collection)
	ids := make([]string, 0, len(cols))
	for _, c := range cols {
		ids = append(ids, c.OwnerID)
	}
	writeJSON(w, http.StatusOK, map[string]any{"collections": cols, "users": s.summaries(ids...)})
}

func (s *Server) handleSocialCollectionCreate(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in social.CreateCollectionInput
	if !s.decode(w, r, &in) {
		return
	}
	in.UserID = v.UserID
	out, err := s.socialCall(contracts.CapSocialCollections, in)
	s.writeCollection(w, http.StatusCreated, out, err)
}

func (s *Server) handleSocialCollectionGet(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, err := s.socialCall(contracts.CapSocialCollections, social.GetCollectionInput{ViewerID: v.UserID, ID: r.PathValue("id")})
	s.writeCollection(w, http.StatusOK, out, err)
}

func (s *Server) handleSocialCollectionPatch(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in social.UpdateCollectionInput
	if !s.decode(w, r, &in) {
		return
	}
	in.UserID, in.ID = v.UserID, r.PathValue("id")
	out, err := s.socialCall(contracts.CapSocialCollections, in)
	s.writeCollection(w, http.StatusOK, out, err)
}

func (s *Server) handleSocialCollectionDelete(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	if _, err := s.socialCall(contracts.CapSocialCollections, social.DeleteCollectionInput{UserID: v.UserID, ID: r.PathValue("id")}); err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]any{"removed": true})
}

func (s *Server) handleSocialCollectionAdd(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		Work contracts.WorkRef `json:"work"`
		Note string            `json:"note"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	work, err := s.resolveWork(in.Work)
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	out, err := s.socialCall(contracts.CapSocialCollections, social.AddCollectionItemInput{UserID: v.UserID, ID: r.PathValue("id"), Work: work, Note: in.Note})
	s.writeCollection(w, http.StatusOK, out, err)
}

func (s *Server) handleSocialCollectionRemove(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	work, err := s.resolveWork(workFromQuery(r))
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	out, err := s.socialCall(contracts.CapSocialCollections, social.RemoveCollectionItemInput{UserID: v.UserID, ID: r.PathValue("id"), Work: work})
	s.writeCollection(w, http.StatusOK, out, err)
}

func (s *Server) handleSocialCollectionShare(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		To      []string `json:"to"`
		Message string   `json:"message"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	out, err := s.socialCall(contracts.CapSocialCollections, social.ShareCollectionInput{UserID: v.UserID, ID: r.PathValue("id"), To: in.To, Message: in.Message})
	if err != nil {
		s.writeSocialErr(w, err)
		return
	}
	writeJSON(w, http.StatusOK, out)
}

func (s *Server) handleSocialCollectionUnshare(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	out, err := s.socialCall(contracts.CapSocialCollections, social.UnshareCollectionInput{UserID: v.UserID, ID: r.PathValue("id"), OtherID: r.PathValue("user")})
	s.writeCollection(w, http.StatusOK, out, err)
}
