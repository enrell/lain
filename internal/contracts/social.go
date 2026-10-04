package contracts

// Capability names for the social domain (docs/slices/social.md). Social
// is its own domain beside catalog, userstate and the list: it never
// writes their records, it only observes progress to record activity.
const (
	// CapSocialGraph owns per-account privacy settings, favorites and
	// relationships (friend requests, friendships, blocks).
	CapSocialGraph = "lain.social.graph@1"
	// CapSocialActivity owns the activity log, the friends feed and
	// notifications.
	CapSocialActivity = "lain.social.activity@1"
	// CapSocialReviews owns ratings, reviews and comments on works.
	CapSocialReviews = "lain.social.reviews@1"
	// CapSocialCollections owns user collections and their sharing.
	CapSocialCollections = "lain.social.collections@1"
)

// Visibility levels. Public means every signed-in account on this
// server; nothing social is anonymous or federated.
const (
	VisibilityPublic  = "public"
	VisibilityFriends = "friends"
	VisibilityPrivate = "private"
)

// Who may send a friend request.
const (
	RequestsEveryone = "everyone"
	RequestsNobody   = "nobody"
)

// Relation is how the viewer stands toward another account. A block
// placed by the other side is never reported: it reads as RelationNone
// in lists and as an unknown user everywhere else.
const (
	RelationNone     = "none"
	RelationFriend   = "friend"
	RelationOutgoing = "outgoing" // the viewer asked
	RelationIncoming = "incoming" // the other account asked
	RelationBlocking = "blocking" // the viewer blocked them
)

// Activity types.
const (
	ActivityProgress  = "progress"  // started or resumed watching/reading
	ActivityCompleted = "completed" // finished an item
	ActivityRated     = "rated"     // rated or reviewed a work
)

// Notification types.
const (
	NotifyFriendRequest    = "friend_request"
	NotifyFriendAccepted   = "friend_accepted"
	NotifyShare            = "share"
	NotifyCollectionShared = "collection_shared"
	NotifyReply            = "reply"
)

// WorkRef names a work independently of media kind: anime, series,
// movies, comics, manga or any later kind. Identity is Kind plus the
// normalized title (the title page's grouping rule); ItemID is an
// optional catalog item to link back to and never part of identity.
type WorkRef struct {
	Kind   string `json:"kind"`
	Title  string `json:"title"`
	ItemID string `json:"item_id,omitempty"`
}

// SocialSettings is one account's privacy configuration plus favorites.
type SocialSettings struct {
	UserID string `json:"user_id"`
	// Profile gates bio, favorites and the collections list.
	Profile string `json:"profile"`
	// Activity gates the account's activity rows in feeds and profile.
	Activity string `json:"activity"`
	// Ratings gates ratings and reviews (title pages, profile, feed).
	Ratings string `json:"ratings"`
	// Discoverable lists the account in user search.
	Discoverable bool `json:"discoverable"`
	// AllowRequests is RequestsEveryone or RequestsNobody.
	AllowRequests string `json:"allow_requests"`
	// HideProgress keeps "watching/reading now" rows out of the feed.
	HideProgress bool      `json:"hide_progress"`
	Favorites    []WorkRef `json:"favorites"`
	UpdatedAt    int64     `json:"updated_at,omitempty"`
}

// DefaultSocialSettings is what an account that never opened its
// privacy settings gets.
func DefaultSocialSettings(userID string) SocialSettings {
	return SocialSettings{
		UserID:        userID,
		Profile:       VisibilityPublic,
		Activity:      VisibilityFriends,
		Ratings:       VisibilityFriends,
		Discoverable:  true,
		AllowRequests: RequestsEveryone,
		Favorites:     []WorkRef{},
	}
}

// Relationship is one edge as seen from its owner.
type Relationship struct {
	UserID    string `json:"user_id"`
	State     string `json:"state"`
	UpdatedAt int64  `json:"updated_at"`
}

// Activity is one row of an account's activity log.
type Activity struct {
	ID      string  `json:"id"`
	UserID  string  `json:"user_id"`
	Type    string  `json:"type"`
	Work    WorkRef `json:"work"`
	Season  int     `json:"season,omitempty"`
	Episode int     `json:"episode,omitempty"`
	Score   int     `json:"score,omitempty"`
	At      int64   `json:"at"`
}

// Notification is one event addressed to an account.
type Notification struct {
	ID           string   `json:"id"`
	UserID       string   `json:"user_id"`
	Type         string   `json:"type"`
	FromUserID   string   `json:"from_user_id"`
	Work         *WorkRef `json:"work,omitempty"`
	CollectionID string   `json:"collection_id,omitempty"`
	CommentID    string   `json:"comment_id,omitempty"`
	Message      string   `json:"message,omitempty"`
	Read         bool     `json:"read"`
	At           int64    `json:"at"`
}

// Rating is one account's score and optional review of a work. Score
// is 1..10; a review may stand alone with Score 0.
type Rating struct {
	UserID    string  `json:"user_id"`
	Work      WorkRef `json:"work"`
	Score     int     `json:"score,omitempty"`
	Review    string  `json:"review,omitempty"`
	Spoiler   bool    `json:"spoiler,omitempty"`
	CreatedAt int64   `json:"created_at"`
	UpdatedAt int64   `json:"updated_at"`
}

// Comment is a public remark on a work, optionally replying to another.
type Comment struct {
	ID        string  `json:"id"`
	UserID    string  `json:"user_id"`
	Work      WorkRef `json:"work"`
	Body      string  `json:"body"`
	ReplyTo   string  `json:"reply_to,omitempty"`
	CreatedAt int64   `json:"created_at"`
}

// CollectionItem is one work in a collection.
type CollectionItem struct {
	Work    WorkRef `json:"work"`
	Note    string  `json:"note,omitempty"`
	AddedAt int64   `json:"added_at"`
}

// Collection is a named, ordered, shareable set of works.
type Collection struct {
	ID          string           `json:"id"`
	OwnerID     string           `json:"owner_id"`
	Name        string           `json:"name"`
	Description string           `json:"description,omitempty"`
	Visibility  string           `json:"visibility"`
	SharedWith  []string         `json:"shared_with"`
	Items       []CollectionItem `json:"items"`
	CreatedAt   int64            `json:"created_at"`
	UpdatedAt   int64            `json:"updated_at"`
}
