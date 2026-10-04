package social

import (
	"encoding/json"
	"sort"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// RecordProgressInput reports one progress write the gateway accepted.
// WasCompleted is the stored state before the write, so completion is
// recorded only on the transition (the D-084 rule).
type RecordProgressInput struct {
	UserID       string            `json:"user_id"`
	Work         contracts.WorkRef `json:"work"`
	Season       int               `json:"season,omitempty"`
	Episode      int               `json:"episode,omitempty"`
	Completed    bool              `json:"completed"`
	WasCompleted bool              `json:"was_completed"`
}

// RecordProgressOutput says which row, if any, was written.
type RecordProgressOutput struct {
	Recorded string `json:"recorded,omitempty"`
}

// FeedInput reads the friends feed, newest first, strictly older than
// Before (unix seconds; 0 reads from now).
type FeedInput struct {
	UserID string `json:"user_id"`
	Before int64  `json:"before,omitempty"`
	Limit  int    `json:"limit,omitempty"`
}

// UserActivityInput reads one account's activity as Viewer may see it.
type UserActivityInput struct {
	ViewerID string `json:"viewer_id"`
	OwnerID  string `json:"owner_id"`
	Before   int64  `json:"before,omitempty"`
	Limit    int    `json:"limit,omitempty"`
}

// NotificationsInput reads an account's notifications, newest first.
type NotificationsInput struct {
	UserID string `json:"user_id"`
	Limit  int    `json:"limit,omitempty"`
}

// NotificationsOutput carries the page and the unread total.
type NotificationsOutput struct {
	Items  []contracts.Notification `json:"items"`
	Unread int                      `json:"unread"`
}

// MarkReadInput marks notifications read: the listed ids, or all.
type MarkReadInput struct {
	UserID string   `json:"user_id"`
	IDs    []string `json:"ids,omitempty"`
	All    bool     `json:"all,omitempty"`
}

func (s *Service) invokeActivity(input any) (any, error) {
	switch in := input.(type) {
	case RecordProgressInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.RecordProgress(in)
	case FeedInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.Feed(in)
	case UserActivityInput:
		if err := needUser(in.ViewerID, in.OwnerID); err != nil {
			return nil, err
		}
		return s.UserActivity(in)
	case NotificationsInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.Notifications(in.UserID, in.Limit)
	case MarkReadInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.MarkRead(in)
	default:
		return nil, invalid("RecordProgressInput, FeedInput, UserActivityInput, NotificationsInput or MarkReadInput required")
	}
}

func decode(raw []byte, v any) error { return json.Unmarshal(raw, v) }

// appendActivityTx writes one row and trims the account to its newest
// maxActivityRows.
func (s *Service) appendActivityTx(tx *bolt.Tx, a contracts.Activity) error {
	a.ID = s.newID()
	if a.At == 0 {
		a.At = s.now().Unix()
	}
	if err := kv.PutJSON(tx, kv.BSocialActivity, pair(a.UserID, a.ID), a); err != nil {
		return err
	}
	return trimPrefix(tx.Bucket(kv.BSocialActivity), prefix(a.UserID), maxActivityRows)
}

// trimPrefix deletes the oldest keys under p beyond keep. Keys under a
// prefix are time-ordered ids, so the oldest come first.
func trimPrefix(b *bolt.Bucket, p []byte, keep int) error {
	var keys [][]byte
	eachPrefix(b, p, false, func(k, _ []byte) bool {
		keys = append(keys, append([]byte(nil), k...))
		return true
	})
	for i := 0; i < len(keys)-keep; i++ {
		if err := b.Delete(keys[i]); err != nil {
			return err
		}
	}
	return nil
}

// RecordProgress turns one progress write into at most one activity
// row: "completed" on the transition, otherwise "progress" once per
// work per window, so a player reporting every few seconds does not
// flood anyone's feed.
func (s *Service) RecordProgress(in RecordProgressInput) (RecordProgressOutput, error) {
	w, err := NormalizeWork(in.Work)
	if err != nil {
		return RecordProgressOutput{}, err
	}
	now := s.now().Unix()
	last := pair(in.UserID, WorkKey(w))
	kind := ""
	switch {
	case in.Completed && !in.WasCompleted:
		kind = contracts.ActivityCompleted
	case !in.Completed:
		var at int64
		_ = s.db.View(func(tx *bolt.Tx) error {
			_ = kv.GetJSON(tx, kv.BSocialActivityLast, last, &at)
			return nil
		})
		if at == 0 || now-at >= int64(progressWindow.Seconds()) {
			kind = contracts.ActivityProgress
		}
	}
	if kind == "" {
		return RecordProgressOutput{}, nil
	}
	err = s.db.Update(func(tx *bolt.Tx) error {
		if err := kv.PutJSON(tx, kv.BSocialActivityLast, last, now); err != nil {
			return err
		}
		return s.appendActivityTx(tx, contracts.Activity{
			UserID: in.UserID, Type: kind, Work: w, Season: in.Season, Episode: in.Episode, At: now,
		})
	})
	if err != nil {
		return RecordProgressOutput{}, err
	}
	return RecordProgressOutput{Recorded: kind}, nil
}

// visibleRow applies the per-row rules on top of activity visibility:
// rating rows also need ratings visibility, and progress rows vanish
// for accounts that hide what they are watching or reading now.
func visibleRow(tx *bolt.Tx, viewer string, st contracts.SocialSettings, a contracts.Activity) bool {
	if viewer == a.UserID {
		return true
	}
	switch a.Type {
	case contracts.ActivityProgress:
		return !st.HideProgress
	case contracts.ActivityRated:
		return levelAllows(tx, st.Ratings, viewer, a.UserID)
	}
	return true
}

// readActivity collects up to limit of owner's rows older than before.
func readActivity(tx *bolt.Tx, viewer, owner string, before int64, limit int) ([]contracts.Activity, error) {
	st, err := getSettings(tx, owner)
	if err != nil {
		return nil, err
	}
	var out []contracts.Activity
	eachPrefix(tx.Bucket(kv.BSocialActivity), prefix(owner), true, func(_, v []byte) bool {
		var a contracts.Activity
		if decode(v, &a) != nil {
			return true
		}
		if before > 0 && a.At >= before {
			return true
		}
		if visibleRow(tx, viewer, st, a) {
			out = append(out, a)
		}
		return len(out) < limit
	})
	return out, nil
}

// Feed merges the activity of every friend whose activity the viewer
// may see, newest first.
func (s *Service) Feed(in FeedInput) ([]contracts.Activity, error) {
	limit := clampLimit(in.Limit)
	out := []contracts.Activity{}
	err := s.db.View(func(tx *bolt.Tx) error {
		for _, f := range friendIDs(tx, in.UserID) {
			ok, err := canSeeTx(tx, in.UserID, f, SubjectActivity)
			if err != nil {
				return err
			}
			if !ok {
				continue
			}
			rows, err := readActivity(tx, in.UserID, f, in.Before, limit)
			if err != nil {
				return err
			}
			out = append(out, rows...)
		}
		return nil
	})
	sortActivity(out)
	if len(out) > limit {
		out = out[:limit]
	}
	return out, err
}

func sortActivity(a []contracts.Activity) {
	sort.SliceStable(a, func(i, j int) bool {
		if a[i].At != a[j].At {
			return a[i].At > a[j].At
		}
		return a[i].ID > a[j].ID
	})
}

// UserActivity reads one account's rows as the viewer may see them;
// a hidden account reads as empty, never as an error that confirms it.
func (s *Service) UserActivity(in UserActivityInput) ([]contracts.Activity, error) {
	out := []contracts.Activity{}
	err := s.db.View(func(tx *bolt.Tx) error {
		ok, err := canSeeTx(tx, in.ViewerID, in.OwnerID, SubjectActivity)
		if err != nil || !ok {
			return err
		}
		rows, err := readActivity(tx, in.ViewerID, in.OwnerID, in.Before, clampLimit(in.Limit))
		if rows != nil {
			out = rows
		}
		return err
	})
	return out, err
}

// notifyTx stores one notification and trims the inbox. It is silent
// toward a recipient who blocked the sender, without telling either.
func (s *Service) notifyTx(tx *bolt.Tx, n contracts.Notification) error {
	if n.FromUserID != "" && blockedEither(tx, n.UserID, n.FromUserID) {
		return nil
	}
	n.ID = s.newID()
	n.At = s.now().Unix()
	n.Read = false
	if err := kv.PutJSON(tx, kv.BSocialNotifications, pair(n.UserID, n.ID), n); err != nil {
		return err
	}
	return trimPrefix(tx.Bucket(kv.BSocialNotifications), prefix(n.UserID), maxNotifications)
}

// Notifications reads the newest notifications and the unread total.
// Rows from accounts now blocked either way are skipped.
func (s *Service) Notifications(userID string, limit int) (NotificationsOutput, error) {
	limit = clampLimit(limit)
	out := NotificationsOutput{Items: []contracts.Notification{}}
	err := s.db.View(func(tx *bolt.Tx) error {
		eachPrefix(tx.Bucket(kv.BSocialNotifications), prefix(userID), true, func(_, v []byte) bool {
			var n contracts.Notification
			if decode(v, &n) != nil {
				return true
			}
			if n.FromUserID != "" && blockedEither(tx, userID, n.FromUserID) {
				return true
			}
			if !n.Read {
				out.Unread++
			}
			if len(out.Items) < limit {
				out.Items = append(out.Items, n)
			}
			return true
		})
		return nil
	})
	return out, err
}

// MarkRead flags the listed notifications (or all) as read and returns
// how many changed.
func (s *Service) MarkRead(in MarkReadInput) (int, error) {
	want := map[string]bool{}
	for _, id := range in.IDs {
		want[id] = true
	}
	changed := 0
	err := s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BSocialNotifications)
		type upd struct {
			k []byte
			n contracts.Notification
		}
		var ups []upd
		eachPrefix(b, prefix(in.UserID), false, func(k, v []byte) bool {
			var n contracts.Notification
			if decode(v, &n) != nil || n.Read || !(in.All || want[n.ID]) {
				return true
			}
			n.Read = true
			ups = append(ups, upd{append([]byte(nil), k...), n})
			return true
		})
		for _, u := range ups {
			if err := kv.PutJSON(tx, kv.BSocialNotifications, u.k, u.n); err != nil {
				return err
			}
			changed++
		}
		return nil
	})
	return changed, err
}
