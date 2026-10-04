package social

import (
	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// Relationship actions.
const (
	ActionRequest = "request" // ask, or accept when they already asked
	ActionAccept  = "accept"
	ActionDecline = "decline"
	ActionRemove  = "remove" // unfriend, or cancel an outgoing request
	ActionBlock   = "block"
	ActionUnblock = "unblock"
)

// Visibility subjects.
const (
	SubjectProfile  = "profile"
	SubjectActivity = "activity"
	SubjectRatings  = "ratings"
)

// GetSettingsInput reads an account's settings (defaults when unset).
type GetSettingsInput struct {
	UserID string `json:"user_id"`
}

// SettingsPatch changes only the fields it carries.
type SettingsPatch struct {
	Profile       *string              `json:"profile"`
	Activity      *string              `json:"activity"`
	Ratings       *string              `json:"ratings"`
	Discoverable  *bool                `json:"discoverable"`
	AllowRequests *string              `json:"allow_requests"`
	HideProgress  *bool                `json:"hide_progress"`
	Favorites     *[]contracts.WorkRef `json:"favorites"`
}

// PatchSettingsInput applies a patch to the owner's settings.
type PatchSettingsInput struct {
	UserID string        `json:"user_id"`
	Patch  SettingsPatch `json:"patch"`
}

// RelationInput reads how UserID stands toward OtherID.
type RelationInput struct {
	UserID  string `json:"user_id"`
	OtherID string `json:"other_id"`
}

// RelationOutput is the viewer's relation. BlockedBy reports a block
// placed by the other side: the gateway uses it to answer "unknown
// user" and never forwards it.
type RelationOutput struct {
	State     string `json:"state"`
	BlockedBy bool   `json:"blocked_by"`
}

// RelateInput changes a relationship.
type RelateInput struct {
	UserID  string `json:"user_id"`
	OtherID string `json:"other_id"`
	Action  string `json:"action"`
}

// ListRelationsInput lists every visible edge of an account.
type ListRelationsInput struct {
	UserID string `json:"user_id"`
}

// CanSeeInput asks whether Viewer may see Owner's Subject.
type CanSeeInput struct {
	ViewerID string `json:"viewer_id"`
	OwnerID  string `json:"owner_id"`
	Subject  string `json:"subject"`
}

// edge is the stored side of a relationship. State is the owner's
// view; BlockedBy is kept apart so a mutual block survives either
// side lifting theirs.
type edge struct {
	State     string `json:"state,omitempty"`
	BlockedBy bool   `json:"blocked_by,omitempty"`
	UpdatedAt int64  `json:"updated_at"`
}

func (s *Service) invokeGraph(input any) (any, error) {
	switch in := input.(type) {
	case GetSettingsInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.Settings(in.UserID)
	case PatchSettingsInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.PatchSettings(in.UserID, in.Patch)
	case RelationInput:
		if err := needUser(in.UserID, in.OtherID); err != nil {
			return nil, err
		}
		return s.Relation(in.UserID, in.OtherID)
	case RelateInput:
		if err := needUser(in.UserID, in.OtherID); err != nil {
			return nil, err
		}
		return s.Relate(in.UserID, in.OtherID, in.Action)
	case ListRelationsInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.Relations(in.UserID)
	case CanSeeInput:
		if err := needUser(in.ViewerID, in.OwnerID); err != nil {
			return nil, err
		}
		return s.CanSee(in.ViewerID, in.OwnerID, in.Subject)
	default:
		return nil, invalid("GetSettingsInput, PatchSettingsInput, RelationInput, RelateInput, ListRelationsInput or CanSeeInput required")
	}
}

func getSettings(tx *bolt.Tx, userID string) (contracts.SocialSettings, error) {
	st := contracts.DefaultSocialSettings(userID)
	err := kv.GetJSON(tx, kv.BSocialSettings, []byte(userID), &st)
	if kv.IsNotFound(err) {
		return contracts.DefaultSocialSettings(userID), nil
	}
	if st.Favorites == nil {
		st.Favorites = []contracts.WorkRef{}
	}
	return st, err
}

// Settings returns the account's settings, defaults when never saved.
func (s *Service) Settings(userID string) (contracts.SocialSettings, error) {
	var out contracts.SocialSettings
	err := s.db.View(func(tx *bolt.Tx) error {
		var err error
		out, err = getSettings(tx, userID)
		return err
	})
	return out, err
}

// PatchSettings validates and applies a settings patch.
func (s *Service) PatchSettings(userID string, p SettingsPatch) (contracts.SocialSettings, error) {
	for _, v := range []*string{p.Profile, p.Activity, p.Ratings} {
		if v != nil && !validVisibility(*v) {
			return contracts.SocialSettings{}, invalid("visibility must be public, friends or private")
		}
	}
	if p.AllowRequests != nil && *p.AllowRequests != contracts.RequestsEveryone && *p.AllowRequests != contracts.RequestsNobody {
		return contracts.SocialSettings{}, invalid("allow_requests must be everyone or nobody")
	}
	var favs []contracts.WorkRef
	if p.Favorites != nil {
		if len(*p.Favorites) > maxFavorites {
			return contracts.SocialSettings{}, invalid("at most 12 favorites")
		}
		seen := map[string]bool{}
		favs = []contracts.WorkRef{}
		for _, w := range *p.Favorites {
			w, err := NormalizeWork(w)
			if err != nil {
				return contracts.SocialSettings{}, err
			}
			if k := WorkKey(w); !seen[k] {
				seen[k] = true
				favs = append(favs, w)
			}
		}
	}
	var out contracts.SocialSettings
	err := s.db.Update(func(tx *bolt.Tx) error {
		st, err := getSettings(tx, userID)
		if err != nil {
			return err
		}
		if p.Profile != nil {
			st.Profile = *p.Profile
		}
		if p.Activity != nil {
			st.Activity = *p.Activity
		}
		if p.Ratings != nil {
			st.Ratings = *p.Ratings
		}
		if p.Discoverable != nil {
			st.Discoverable = *p.Discoverable
		}
		if p.AllowRequests != nil {
			st.AllowRequests = *p.AllowRequests
		}
		if p.HideProgress != nil {
			st.HideProgress = *p.HideProgress
		}
		if favs != nil {
			st.Favorites = favs
		}
		st.UserID = userID
		st.UpdatedAt = s.now().Unix()
		out = st
		return kv.PutJSON(tx, kv.BSocialSettings, []byte(userID), st)
	})
	return out, err
}

func getEdge(tx *bolt.Tx, owner, other string) edge {
	var e edge
	_ = kv.GetJSON(tx, kv.BSocialEdges, pair(owner, other), &e)
	return e
}

func putEdge(tx *bolt.Tx, owner, other string, e edge) error {
	if e.State == "" && !e.BlockedBy {
		return tx.Bucket(kv.BSocialEdges).Delete(pair(owner, other))
	}
	return kv.PutJSON(tx, kv.BSocialEdges, pair(owner, other), e)
}

func relationOf(e edge) RelationOutput {
	st := e.State
	if st == "" {
		st = contracts.RelationNone
	}
	return RelationOutput{State: st, BlockedBy: e.BlockedBy}
}

// Relation reports how userID stands toward otherID.
func (s *Service) Relation(userID, otherID string) (RelationOutput, error) {
	var out RelationOutput
	err := s.db.View(func(tx *bolt.Tx) error {
		out = relationOf(getEdge(tx, userID, otherID))
		return nil
	})
	return out, err
}

// blockedEither reports a block in either direction.
func blockedEither(tx *bolt.Tx, a, b string) bool {
	e := getEdge(tx, a, b)
	return e.State == contracts.RelationBlocking || e.BlockedBy
}

func areFriends(tx *bolt.Tx, a, b string) bool {
	return getEdge(tx, a, b).State == contracts.RelationFriend
}

// Relate applies one relationship action and returns the new relation
// from the actor's side. Friend requests and acceptances notify the
// other account inside the same transaction.
func (s *Service) Relate(userID, otherID, action string) (RelationOutput, error) {
	if userID == otherID {
		return RelationOutput{}, invalid("cannot relate to yourself")
	}
	var out RelationOutput
	err := s.db.Update(func(tx *bolt.Tx) error {
		now := s.now().Unix()
		mine, theirs := getEdge(tx, userID, otherID), getEdge(tx, otherID, userID)
		switch action {
		case ActionRequest, ActionAccept:
			if mine.BlockedBy {
				// Never reveal the block: the account simply is not there.
				return notFoundErr("unknown user")
			}
			if mine.State == contracts.RelationBlocking {
				return forbidden("unblock this account first")
			}
			switch mine.State {
			case contracts.RelationFriend, contracts.RelationOutgoing:
				// Idempotent.
			case contracts.RelationIncoming:
				mine.State, theirs.State = contracts.RelationFriend, contracts.RelationFriend
				if err := settleRequestTx(tx, userID, otherID); err != nil {
					return err
				}
				if err := s.notifyTx(tx, contracts.Notification{UserID: otherID, Type: contracts.NotifyFriendAccepted, FromUserID: userID}); err != nil {
					return err
				}
			default:
				if action == ActionAccept {
					return notFoundErr("no pending request from this account")
				}
				st, err := getSettings(tx, otherID)
				if err != nil {
					return err
				}
				if st.AllowRequests == contracts.RequestsNobody {
					return forbidden("this account does not accept friend requests")
				}
				mine.State, theirs.State = contracts.RelationOutgoing, contracts.RelationIncoming
				if err := s.notifyTx(tx, contracts.Notification{UserID: otherID, Type: contracts.NotifyFriendRequest, FromUserID: userID}); err != nil {
					return err
				}
			}
		case ActionDecline:
			if mine.State != contracts.RelationIncoming {
				return notFoundErr("no pending request from this account")
			}
			mine.State, theirs.State = "", ""
			if err := settleRequestTx(tx, userID, otherID); err != nil {
				return err
			}
		case ActionRemove:
			switch mine.State {
			case contracts.RelationOutgoing:
				// A cancelled request must not stay actionable in their inbox.
				if err := settleRequestTx(tx, otherID, userID); err != nil {
					return err
				}
				mine.State, theirs.State = "", ""
			case contracts.RelationFriend, contracts.RelationIncoming:
				mine.State, theirs.State = "", ""
			}
		case ActionBlock:
			mine.State = contracts.RelationBlocking
			if theirs.State != contracts.RelationBlocking {
				theirs.State = ""
			}
			theirs.BlockedBy = true
		case ActionUnblock:
			if mine.State == contracts.RelationBlocking {
				mine.State = ""
				theirs.BlockedBy = false
			}
		default:
			return invalid("unknown relationship action")
		}
		mine.UpdatedAt, theirs.UpdatedAt = now, now
		if err := putEdge(tx, userID, otherID, mine); err != nil {
			return err
		}
		if err := putEdge(tx, otherID, userID, theirs); err != nil {
			return err
		}
		out = relationOf(mine)
		return nil
	})
	return out, err
}

// Relations lists the account's edges as it may see them: friends,
// pending requests both ways and its own blocks. Blocks placed on it
// are omitted.
func (s *Service) Relations(userID string) ([]contracts.Relationship, error) {
	out := []contracts.Relationship{}
	err := s.db.View(func(tx *bolt.Tx) error {
		p := prefix(userID)
		eachPrefix(tx.Bucket(kv.BSocialEdges), p, false, func(k, v []byte) bool {
			var e edge
			if err := decode(v, &e); err != nil || e.State == "" {
				return true
			}
			out = append(out, contracts.Relationship{UserID: string(k[len(p):]), State: e.State, UpdatedAt: e.UpdatedAt})
			return true
		})
		return nil
	})
	return out, err
}

// Friends returns the ids of the account's friends.
func friendIDs(tx *bolt.Tx, userID string) []string {
	var out []string
	p := prefix(userID)
	eachPrefix(tx.Bucket(kv.BSocialEdges), p, false, func(k, v []byte) bool {
		var e edge
		if decode(v, &e) == nil && e.State == contracts.RelationFriend {
			out = append(out, string(k[len(p):]))
		}
		return true
	})
	return out
}

// canSeeTx is the one privacy rule: owners see themselves, a block in
// either direction hides everything, then the owner's level decides.
func canSeeTx(tx *bolt.Tx, viewer, owner, subject string) (bool, error) {
	if viewer == owner {
		return true, nil
	}
	if blockedEither(tx, viewer, owner) {
		return false, nil
	}
	st, err := getSettings(tx, owner)
	if err != nil {
		return false, err
	}
	var level string
	switch subject {
	case SubjectProfile:
		level = st.Profile
	case SubjectActivity:
		level = st.Activity
	case SubjectRatings:
		level = st.Ratings
	default:
		return false, invalid("unknown visibility subject")
	}
	return levelAllows(tx, level, viewer, owner), nil
}

func levelAllows(tx *bolt.Tx, level, viewer, owner string) bool {
	switch level {
	case contracts.VisibilityPublic:
		return true
	case contracts.VisibilityFriends:
		return areFriends(tx, viewer, owner)
	default:
		return false
	}
}

// CanSee reports whether viewer may see owner's subject.
func (s *Service) CanSee(viewer, owner, subject string) (bool, error) {
	var ok bool
	err := s.db.View(func(tx *bolt.Tx) error {
		var err error
		ok, err = canSeeTx(tx, viewer, owner, subject)
		return err
	})
	return ok, err
}

// settleRequestTx marks recipient's friend-request notifications from
// sender read once the request is answered or withdrawn, so an inbox
// never offers to accept a request that no longer exists.
func settleRequestTx(tx *bolt.Tx, recipient, sender string) error {
	b := tx.Bucket(kv.BSocialNotifications)
	var keys [][]byte
	var rows []contracts.Notification
	eachPrefix(b, prefix(recipient), false, func(k, v []byte) bool {
		var n contracts.Notification
		if decode(v, &n) == nil && !n.Read && n.Type == contracts.NotifyFriendRequest && n.FromUserID == sender {
			n.Read = true
			keys = append(keys, append([]byte(nil), k...))
			rows = append(rows, n)
		}
		return true
	})
	for i, k := range keys {
		if err := kv.PutJSON(tx, kv.BSocialNotifications, k, rows[i]); err != nil {
			return err
		}
	}
	return nil
}
