package kv

// Social-domain buckets (docs/slices/social.md). They live in their own
// file and are created by the social provider itself, so the shared
// bucket list in Open stays untouched by the slice.
var (
	BSocialSettings       = []byte("social_settings")
	BSocialEdges          = []byte("social_edges")
	BSocialActivity       = []byte("social_activity")
	BSocialActivityLast   = []byte("social_activity_last")
	BSocialNotifications  = []byte("social_notifications")
	BSocialRatings        = []byte("social_ratings")
	BSocialRatingsByUser  = []byte("social_ratings_by_user")
	BSocialComments       = []byte("social_comments")
	BSocialCommentsByWork = []byte("social_comments_by_work")
	BSocialCollections    = []byte("social_collections")
)

// SocialBuckets lists every social bucket for creation and backup.
func SocialBuckets() [][]byte {
	return [][]byte{
		BSocialSettings, BSocialEdges, BSocialActivity, BSocialActivityLast,
		BSocialNotifications, BSocialRatings, BSocialRatingsByUser,
		BSocialComments, BSocialCommentsByWork, BSocialCollections,
	}
}
