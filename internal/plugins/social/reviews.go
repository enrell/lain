package social

import (
	"sort"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// WorkInput reads the social view of a work for a viewer.
type WorkInput struct {
	ViewerID string            `json:"viewer_id"`
	Work     contracts.WorkRef `json:"work"`
}

// WorkView is everything the viewer may see about a work: their own
// rating, other visible ratings with the visible average, and comments.
type WorkView struct {
	Work     contracts.WorkRef   `json:"work"`
	Mine     *contracts.Rating   `json:"mine,omitempty"`
	Ratings  []contracts.Rating  `json:"ratings"`
	Average  float64             `json:"average"`
	Scored   int                 `json:"scored"`
	Comments []contracts.Comment `json:"comments"`
}

// PutRatingInput sets the account's score and/or review of a work.
type PutRatingInput struct {
	UserID  string            `json:"user_id"`
	Work    contracts.WorkRef `json:"work"`
	Score   int               `json:"score"`
	Review  string            `json:"review"`
	Spoiler bool              `json:"spoiler"`
}

// DeleteRatingInput removes the account's rating of a work.
type DeleteRatingInput struct {
	UserID string            `json:"user_id"`
	Work   contracts.WorkRef `json:"work"`
}

// UserRatingsInput lists an account's ratings as the viewer may see them.
type UserRatingsInput struct {
	ViewerID string `json:"viewer_id"`
	OwnerID  string `json:"owner_id"`
}

// AddCommentInput posts a comment on a work.
type AddCommentInput struct {
	UserID  string            `json:"user_id"`
	Work    contracts.WorkRef `json:"work"`
	Body    string            `json:"body"`
	ReplyTo string            `json:"reply_to,omitempty"`
}

// DeleteCommentInput removes a comment; authors delete their own and
// moderators (admins) any.
type DeleteCommentInput struct {
	UserID    string `json:"user_id"`
	ID        string `json:"id"`
	Moderator bool   `json:"moderator,omitempty"`
}

func (s *Service) invokeReviews(input any) (any, error) {
	switch in := input.(type) {
	case WorkInput:
		if err := needUser(in.ViewerID); err != nil {
			return nil, err
		}
		return s.Work(in.ViewerID, in.Work)
	case PutRatingInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.PutRating(in)
	case DeleteRatingInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.DeleteRating(in.UserID, in.Work)
	case UserRatingsInput:
		if err := needUser(in.ViewerID, in.OwnerID); err != nil {
			return nil, err
		}
		return s.UserRatings(in.ViewerID, in.OwnerID)
	case AddCommentInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.AddComment(in)
	case DeleteCommentInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.DeleteComment(in)
	default:
		return nil, invalid("WorkInput, PutRatingInput, DeleteRatingInput, UserRatingsInput, AddCommentInput or DeleteCommentInput required")
	}
}

// Work assembles the viewer's view of a work.
func (s *Service) Work(viewer string, w contracts.WorkRef) (WorkView, error) {
	w, err := NormalizeWork(w)
	if err != nil {
		return WorkView{}, err
	}
	key := WorkKey(w)
	out := WorkView{Work: w, Ratings: []contracts.Rating{}, Comments: []contracts.Comment{}}
	err = s.db.View(func(tx *bolt.Tx) error {
		sum := 0
		var ferr error
		eachPrefix(tx.Bucket(kv.BSocialRatings), prefix(key), false, func(_, v []byte) bool {
			var r contracts.Rating
			if decode(v, &r) != nil {
				return true
			}
			if r.UserID == viewer {
				mine := r
				out.Mine = &mine
			} else {
				ok, err := canSeeTx(tx, viewer, r.UserID, SubjectRatings)
				if err != nil {
					ferr = err
					return false
				}
				if !ok {
					return true
				}
				out.Ratings = append(out.Ratings, r)
			}
			if r.Score > 0 {
				sum += r.Score
				out.Scored++
			}
			return true
		})
		if ferr != nil {
			return ferr
		}
		if out.Scored > 0 {
			out.Average = float64(sum) / float64(out.Scored)
		}
		sort.SliceStable(out.Ratings, func(i, j int) bool { return out.Ratings[i].UpdatedAt > out.Ratings[j].UpdatedAt })
		eachPrefix(tx.Bucket(kv.BSocialCommentsByWork), prefix(key), false, func(k, _ []byte) bool {
			var c contracts.Comment
			if kv.GetJSON(tx, kv.BSocialComments, k[len(key)+1:], &c) != nil {
				return true
			}
			if c.UserID != viewer && blockedEither(tx, viewer, c.UserID) {
				return true
			}
			out.Comments = append(out.Comments, c)
			return true
		})
		return nil
	})
	return out, err
}

// PutRating stores the account's rating. A changed score or a new
// review records a "rated" activity row.
func (s *Service) PutRating(in PutRatingInput) (contracts.Rating, error) {
	w, err := NormalizeWork(in.Work)
	if err != nil {
		return contracts.Rating{}, err
	}
	if in.Score < 0 || in.Score > 10 {
		return contracts.Rating{}, invalid("score must be 1..10, or 0 for a review without a score")
	}
	review, err := cleanText(in.Review, maxReview, "review", true)
	if err != nil {
		return contracts.Rating{}, err
	}
	if in.Score == 0 && review == "" {
		return contracts.Rating{}, invalid("a rating needs a score or a review")
	}
	key := WorkKey(w)
	var out contracts.Rating
	err = s.db.Update(func(tx *bolt.Tx) error {
		now := s.now().Unix()
		var prev contracts.Rating
		had := kv.GetJSON(tx, kv.BSocialRatings, pair(key, in.UserID), &prev) == nil
		r := contracts.Rating{UserID: in.UserID, Work: w, Score: in.Score, Review: review, Spoiler: in.Spoiler, CreatedAt: now, UpdatedAt: now}
		if had {
			r.CreatedAt = prev.CreatedAt
		}
		if err := kv.PutJSON(tx, kv.BSocialRatings, pair(key, in.UserID), r); err != nil {
			return err
		}
		if err := tx.Bucket(kv.BSocialRatingsByUser).Put(pair(in.UserID, key), []byte{}); err != nil {
			return err
		}
		if !had || prev.Score != r.Score || (prev.Review == "" && r.Review != "") {
			if err := s.appendActivityTx(tx, contracts.Activity{UserID: in.UserID, Type: contracts.ActivityRated, Work: w, Score: r.Score, At: now}); err != nil {
				return err
			}
		}
		out = r
		return nil
	})
	return out, err
}

// DeleteRating removes the account's rating; absent is not an error.
func (s *Service) DeleteRating(userID string, w contracts.WorkRef) (bool, error) {
	w, err := NormalizeWork(w)
	if err != nil {
		return false, err
	}
	key := WorkKey(w)
	found := false
	err = s.db.Update(func(tx *bolt.Tx) error {
		b := tx.Bucket(kv.BSocialRatings)
		found = b.Get(pair(key, userID)) != nil
		if err := b.Delete(pair(key, userID)); err != nil {
			return err
		}
		return tx.Bucket(kv.BSocialRatingsByUser).Delete(pair(userID, key))
	})
	return found, err
}

// UserRatings lists an account's ratings, newest first, when the
// viewer may see them; otherwise empty.
func (s *Service) UserRatings(viewer, owner string) ([]contracts.Rating, error) {
	out := []contracts.Rating{}
	err := s.db.View(func(tx *bolt.Tx) error {
		ok, err := canSeeTx(tx, viewer, owner, SubjectRatings)
		if err != nil || !ok {
			return err
		}
		p := prefix(owner)
		eachPrefix(tx.Bucket(kv.BSocialRatingsByUser), p, false, func(k, _ []byte) bool {
			var r contracts.Rating
			if kv.GetJSON(tx, kv.BSocialRatings, pair(string(k[len(p):]), owner), &r) == nil {
				out = append(out, r)
			}
			return true
		})
		return nil
	})
	sort.SliceStable(out, func(i, j int) bool { return out[i].UpdatedAt > out[j].UpdatedAt })
	return out, err
}

// AddComment posts a comment. A reply must name a comment on the same
// work and notifies that comment's author.
func (s *Service) AddComment(in AddCommentInput) (contracts.Comment, error) {
	w, err := NormalizeWork(in.Work)
	if err != nil {
		return contracts.Comment{}, err
	}
	body, err := cleanText(in.Body, maxComment, "comment", true)
	if err != nil {
		return contracts.Comment{}, err
	}
	if body == "" {
		return contracts.Comment{}, invalid("comment is empty")
	}
	key := WorkKey(w)
	var out contracts.Comment
	err = s.db.Update(func(tx *bolt.Tx) error {
		c := contracts.Comment{ID: s.newID(), UserID: in.UserID, Work: w, Body: body, CreatedAt: s.now().Unix()}
		var parent contracts.Comment
		if in.ReplyTo != "" {
			if kv.GetJSON(tx, kv.BSocialComments, []byte(in.ReplyTo), &parent) != nil || WorkKey(parent.Work) != key {
				return notFoundErr("unknown comment")
			}
			if blockedEither(tx, in.UserID, parent.UserID) {
				return notFoundErr("unknown comment")
			}
			c.ReplyTo = parent.ID
		}
		if err := kv.PutJSON(tx, kv.BSocialComments, []byte(c.ID), c); err != nil {
			return err
		}
		if err := tx.Bucket(kv.BSocialCommentsByWork).Put(pair(key, c.ID), []byte{}); err != nil {
			return err
		}
		if c.ReplyTo != "" && parent.UserID != in.UserID {
			ref := w
			if err := s.notifyTx(tx, contracts.Notification{UserID: parent.UserID, Type: contracts.NotifyReply, FromUserID: in.UserID, Work: &ref, CommentID: c.ID}); err != nil {
				return err
			}
		}
		out = c
		return nil
	})
	return out, err
}

// DeleteComment removes a comment. Replies stay, pointing at a comment
// that no longer exists; clients render them as replies to a removed one.
func (s *Service) DeleteComment(in DeleteCommentInput) (bool, error) {
	err := s.db.Update(func(tx *bolt.Tx) error {
		var c contracts.Comment
		if kv.GetJSON(tx, kv.BSocialComments, []byte(in.ID), &c) != nil {
			return notFoundErr("unknown comment")
		}
		if c.UserID != in.UserID && !in.Moderator {
			return forbidden("only the author or an admin can delete a comment")
		}
		if err := tx.Bucket(kv.BSocialComments).Delete([]byte(in.ID)); err != nil {
			return err
		}
		return tx.Bucket(kv.BSocialCommentsByWork).Delete(pair(WorkKey(c.Work), c.ID))
	})
	return err == nil, err
}
