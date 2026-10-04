package social

import (
	"sort"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// ListCollectionsInput lists collections. With OwnerID empty it reads
// the viewer's own collections plus those shared with them; otherwise
// the owner's collections the viewer may open.
type ListCollectionsInput struct {
	ViewerID string `json:"viewer_id"`
	OwnerID  string `json:"owner_id,omitempty"`
}

// GetCollectionInput opens one collection.
type GetCollectionInput struct {
	ViewerID string `json:"viewer_id"`
	ID       string `json:"id"`
}

// CreateCollectionInput creates a collection owned by UserID.
type CreateCollectionInput struct {
	UserID      string `json:"user_id"`
	Name        string `json:"name"`
	Description string `json:"description"`
	Visibility  string `json:"visibility"`
}

// UpdateCollectionInput changes only the fields it carries.
type UpdateCollectionInput struct {
	UserID      string  `json:"user_id"`
	ID          string  `json:"id"`
	Name        *string `json:"name"`
	Description *string `json:"description"`
	Visibility  *string `json:"visibility"`
}

// DeleteCollectionInput removes a collection.
type DeleteCollectionInput struct {
	UserID string `json:"user_id"`
	ID     string `json:"id"`
}

// AddCollectionItemInput appends a work (or updates its note).
type AddCollectionItemInput struct {
	UserID string            `json:"user_id"`
	ID     string            `json:"id"`
	Work   contracts.WorkRef `json:"work"`
	Note   string            `json:"note"`
}

// RemoveCollectionItemInput removes a work.
type RemoveCollectionItemInput struct {
	UserID string            `json:"user_id"`
	ID     string            `json:"id"`
	Work   contracts.WorkRef `json:"work"`
}

// ShareCollectionInput gives friends read access and notifies them.
type ShareCollectionInput struct {
	UserID  string   `json:"user_id"`
	ID      string   `json:"id"`
	To      []string `json:"to"`
	Message string   `json:"message"`
}

// UnshareCollectionInput revokes one account's explicit access.
type UnshareCollectionInput struct {
	UserID  string `json:"user_id"`
	ID      string `json:"id"`
	OtherID string `json:"other_id"`
}

// ShareWorkInput recommends a work to friends.
type ShareWorkInput struct {
	UserID  string            `json:"user_id"`
	To      []string          `json:"to"`
	Work    contracts.WorkRef `json:"work"`
	Message string            `json:"message"`
}

// ShareOutput reports who received the share.
type ShareOutput struct {
	Sent []string `json:"sent"`
}

func (s *Service) invokeCollections(input any) (any, error) {
	switch in := input.(type) {
	case ListCollectionsInput:
		if err := needUser(in.ViewerID); err != nil {
			return nil, err
		}
		return s.Collections(in.ViewerID, in.OwnerID)
	case GetCollectionInput:
		if err := needUser(in.ViewerID); err != nil {
			return nil, err
		}
		return s.Collection(in.ViewerID, in.ID)
	case CreateCollectionInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.CreateCollection(in)
	case UpdateCollectionInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.UpdateCollection(in)
	case DeleteCollectionInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.DeleteCollection(in.UserID, in.ID)
	case AddCollectionItemInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.AddCollectionItem(in)
	case RemoveCollectionItemInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.RemoveCollectionItem(in)
	case ShareCollectionInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.ShareCollection(in)
	case UnshareCollectionInput:
		if err := needUser(in.UserID, in.OtherID); err != nil {
			return nil, err
		}
		return s.UnshareCollection(in)
	case ShareWorkInput:
		if err := needUser(in.UserID); err != nil {
			return nil, err
		}
		return s.ShareWork(in)
	default:
		return nil, invalid("a collection or share input is required")
	}
}

// canOpenTx: the owner always; nobody across a block; explicit shares
// while still friends; then the collection's own visibility.
func canOpenTx(tx *bolt.Tx, viewer string, c contracts.Collection) bool {
	if viewer == c.OwnerID {
		return true
	}
	if blockedEither(tx, viewer, c.OwnerID) {
		return false
	}
	friends := areFriends(tx, viewer, c.OwnerID)
	if friends {
		for _, id := range c.SharedWith {
			if id == viewer {
				return true
			}
		}
	}
	return levelAllows(tx, c.Visibility, viewer, c.OwnerID)
}

func getCollection(tx *bolt.Tx, id string) (contracts.Collection, error) {
	var c contracts.Collection
	if id == "" || kv.GetJSON(tx, kv.BSocialCollections, []byte(id), &c) != nil {
		return c, notFoundErr("unknown collection")
	}
	if c.Items == nil {
		c.Items = []contracts.CollectionItem{}
	}
	if c.SharedWith == nil {
		c.SharedWith = []string{}
	}
	return c, nil
}

// redact hides the share list from everyone but the owner.
func redact(viewer string, c contracts.Collection) contracts.Collection {
	if viewer != c.OwnerID {
		c.SharedWith = []string{}
	}
	return c
}

// Collections lists collections for a viewer (see ListCollectionsInput).
func (s *Service) Collections(viewer, owner string) ([]contracts.Collection, error) {
	out := []contracts.Collection{}
	err := s.db.View(func(tx *bolt.Tx) error {
		if owner != "" && owner != viewer {
			// The profile gate covers the list of collections; each
			// collection's own visibility still applies on top.
			ok, err := canSeeTx(tx, viewer, owner, SubjectProfile)
			if err != nil || !ok {
				return err
			}
		}
		return tx.Bucket(kv.BSocialCollections).ForEach(func(_, v []byte) error {
			var c contracts.Collection
			if decode(v, &c) != nil {
				return nil
			}
			if c.Items == nil {
				c.Items = []contracts.CollectionItem{}
			}
			if c.SharedWith == nil {
				c.SharedWith = []string{}
			}
			switch {
			case owner == "" && c.OwnerID == viewer:
				out = append(out, c)
			case owner == "":
				for _, id := range c.SharedWith {
					if id == viewer && canOpenTx(tx, viewer, c) {
						out = append(out, redact(viewer, c))
						break
					}
				}
			case c.OwnerID == owner && canOpenTx(tx, viewer, c):
				out = append(out, redact(viewer, c))
			}
			return nil
		})
	})
	sort.SliceStable(out, func(i, j int) bool { return out[i].UpdatedAt > out[j].UpdatedAt })
	return out, err
}

// Collection opens one collection; a hidden one reads as unknown.
func (s *Service) Collection(viewer, id string) (contracts.Collection, error) {
	var out contracts.Collection
	err := s.db.View(func(tx *bolt.Tx) error {
		c, err := getCollection(tx, id)
		if err != nil {
			return err
		}
		if !canOpenTx(tx, viewer, c) {
			return notFoundErr("unknown collection")
		}
		out = redact(viewer, c)
		return nil
	})
	return out, err
}

func cleanCollectionFields(name, desc *string, vis *string) error {
	if name != nil {
		v, err := cleanText(*name, maxCollectionName, "name", false)
		if err != nil {
			return err
		}
		if v == "" {
			return invalid("name required")
		}
		*name = v
	}
	if desc != nil {
		v, err := cleanText(*desc, maxCollectionDesc, "description", true)
		if err != nil {
			return err
		}
		*desc = v
	}
	if vis != nil && !validVisibility(*vis) {
		return invalid("visibility must be public, friends or private")
	}
	return nil
}

// CreateCollection creates a collection (private unless told otherwise).
func (s *Service) CreateCollection(in CreateCollectionInput) (contracts.Collection, error) {
	if in.Visibility == "" {
		in.Visibility = contracts.VisibilityPrivate
	}
	if err := cleanCollectionFields(&in.Name, &in.Description, &in.Visibility); err != nil {
		return contracts.Collection{}, err
	}
	now := s.now().Unix()
	c := contracts.Collection{
		ID: s.newID(), OwnerID: in.UserID, Name: in.Name, Description: in.Description,
		Visibility: in.Visibility, SharedWith: []string{}, Items: []contracts.CollectionItem{},
		CreatedAt: now, UpdatedAt: now,
	}
	err := s.db.Update(func(tx *bolt.Tx) error {
		return kv.PutJSON(tx, kv.BSocialCollections, []byte(c.ID), c)
	})
	return c, err
}

// mutateOwned loads a collection the user owns, applies fn and saves.
// Someone else's collection reads as unknown when they cannot open it
// and as forbidden when they can.
func (s *Service) mutateOwned(userID, id string, fn func(tx *bolt.Tx, c *contracts.Collection) error) (contracts.Collection, error) {
	var out contracts.Collection
	err := s.db.Update(func(tx *bolt.Tx) error {
		c, err := getCollection(tx, id)
		if err != nil {
			return err
		}
		if c.OwnerID != userID {
			if canOpenTx(tx, userID, c) {
				return forbidden("only the owner can change a collection")
			}
			return notFoundErr("unknown collection")
		}
		if err := fn(tx, &c); err != nil {
			return err
		}
		c.UpdatedAt = s.now().Unix()
		out = c
		return kv.PutJSON(tx, kv.BSocialCollections, []byte(c.ID), c)
	})
	return out, err
}

// UpdateCollection edits name, description or visibility.
func (s *Service) UpdateCollection(in UpdateCollectionInput) (contracts.Collection, error) {
	if err := cleanCollectionFields(in.Name, in.Description, in.Visibility); err != nil {
		return contracts.Collection{}, err
	}
	return s.mutateOwned(in.UserID, in.ID, func(_ *bolt.Tx, c *contracts.Collection) error {
		if in.Name != nil {
			c.Name = *in.Name
		}
		if in.Description != nil {
			c.Description = *in.Description
		}
		if in.Visibility != nil {
			c.Visibility = *in.Visibility
		}
		return nil
	})
}

// DeleteCollection removes a collection the user owns.
func (s *Service) DeleteCollection(userID, id string) (bool, error) {
	err := s.db.Update(func(tx *bolt.Tx) error {
		c, err := getCollection(tx, id)
		if err != nil {
			return err
		}
		if c.OwnerID != userID {
			return notFoundErr("unknown collection")
		}
		return tx.Bucket(kv.BSocialCollections).Delete([]byte(id))
	})
	return err == nil, err
}

// AddCollectionItem appends a work, or updates the note of one already
// in the collection without moving it.
func (s *Service) AddCollectionItem(in AddCollectionItemInput) (contracts.Collection, error) {
	w, err := NormalizeWork(in.Work)
	if err != nil {
		return contracts.Collection{}, err
	}
	note, err := cleanText(in.Note, maxMessage, "note", false)
	if err != nil {
		return contracts.Collection{}, err
	}
	return s.mutateOwned(in.UserID, in.ID, func(_ *bolt.Tx, c *contracts.Collection) error {
		key := WorkKey(w)
		for i := range c.Items {
			if WorkKey(c.Items[i].Work) == key {
				c.Items[i].Note = note
				return nil
			}
		}
		if len(c.Items) >= maxCollectionSize {
			return invalid("a collection holds at most 500 works")
		}
		c.Items = append(c.Items, contracts.CollectionItem{Work: w, Note: note, AddedAt: s.now().Unix()})
		return nil
	})
}

// RemoveCollectionItem removes a work; absent is not an error.
func (s *Service) RemoveCollectionItem(in RemoveCollectionItemInput) (contracts.Collection, error) {
	w, err := NormalizeWork(in.Work)
	if err != nil {
		return contracts.Collection{}, err
	}
	return s.mutateOwned(in.UserID, in.ID, func(_ *bolt.Tx, c *contracts.Collection) error {
		key := WorkKey(w)
		kept := c.Items[:0]
		for _, it := range c.Items {
			if WorkKey(it.Work) != key {
				kept = append(kept, it)
			}
		}
		c.Items = kept
		return nil
	})
}

// friendTargets validates share recipients: friends only, deduplicated,
// never the sender.
func friendTargets(tx *bolt.Tx, userID string, to []string) ([]string, error) {
	if len(to) == 0 {
		return nil, invalid("choose at least one friend")
	}
	if len(to) > maxShareTargets {
		return nil, invalid("too many recipients")
	}
	seen := map[string]bool{}
	var out []string
	for _, id := range to {
		if id == userID || seen[id] {
			continue
		}
		seen[id] = true
		if !areFriends(tx, userID, id) {
			return nil, forbidden("you can only share with friends")
		}
		out = append(out, id)
	}
	if len(out) == 0 {
		return nil, invalid("choose at least one friend")
	}
	return out, nil
}

// ShareCollection grants friends read access and notifies them.
func (s *Service) ShareCollection(in ShareCollectionInput) (ShareOutput, error) {
	msg, err := cleanText(in.Message, maxMessage, "message", false)
	if err != nil {
		return ShareOutput{}, err
	}
	var sent []string
	_, err = s.mutateOwned(in.UserID, in.ID, func(tx *bolt.Tx, c *contracts.Collection) error {
		targets, err := friendTargets(tx, in.UserID, in.To)
		if err != nil {
			return err
		}
		have := map[string]bool{}
		for _, id := range c.SharedWith {
			have[id] = true
		}
		for _, id := range targets {
			if !have[id] {
				c.SharedWith = append(c.SharedWith, id)
			}
			if err := s.notifyTx(tx, contracts.Notification{UserID: id, Type: contracts.NotifyCollectionShared, FromUserID: in.UserID, CollectionID: c.ID, Message: msg}); err != nil {
				return err
			}
		}
		sent = targets
		return nil
	})
	return ShareOutput{Sent: sent}, err
}

// UnshareCollection revokes one account's explicit access.
func (s *Service) UnshareCollection(in UnshareCollectionInput) (contracts.Collection, error) {
	return s.mutateOwned(in.UserID, in.ID, func(_ *bolt.Tx, c *contracts.Collection) error {
		kept := c.SharedWith[:0]
		for _, id := range c.SharedWith {
			if id != in.OtherID {
				kept = append(kept, id)
			}
		}
		c.SharedWith = kept
		return nil
	})
}

// ShareWork recommends a work to friends through their notifications.
func (s *Service) ShareWork(in ShareWorkInput) (ShareOutput, error) {
	w, err := NormalizeWork(in.Work)
	if err != nil {
		return ShareOutput{}, err
	}
	msg, err := cleanText(in.Message, maxMessage, "message", false)
	if err != nil {
		return ShareOutput{}, err
	}
	var sent []string
	err = s.db.Update(func(tx *bolt.Tx) error {
		targets, err := friendTargets(tx, in.UserID, in.To)
		if err != nil {
			return err
		}
		for _, id := range targets {
			ref := w
			if err := s.notifyTx(tx, contracts.Notification{UserID: id, Type: contracts.NotifyShare, FromUserID: in.UserID, Work: &ref, Message: msg}); err != nil {
				return err
			}
		}
		sent = targets
		return nil
	})
	return ShareOutput{Sent: sent}, err
}
