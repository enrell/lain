package auth

import (
	"errors"
	"strings"
	"time"
	"unicode"
	"unicode/utf8"

	bolt "go.etcd.io/bbolt"

	"github.com/enrell/lain/internal/kv"
)

// Avatar kinds (D-086). An uploaded image lives on disk under the data
// directory, never in the account record; the record only says which
// kind is active and a version that changes on every new picture, so
// clients can cache the image URL forever and still see updates.
const (
	AvatarNone   = ""
	AvatarMascot = "mascot"
	AvatarUpload = "upload"
)

// Mascots is the pre-made avatar set. The artwork lives in the web
// client; the server only checks the id so a profile never points at a
// mascot no client can draw.
var Mascots = []string{"wired", "moth", "static", "orbit", "glyph", "shell", "neon", "void"}

const (
	maxDisplayName = 40
	maxBio         = 160
)

// Profile is the public face of an account.
type Profile struct {
	DisplayName string `json:"display_name,omitempty"`
	Bio         string `json:"bio,omitempty"`
	Avatar      Avatar `json:"avatar"`
}

// Avatar selects the picture shown for an account.
type Avatar struct {
	Kind    string `json:"kind,omitempty"`
	Mascot  string `json:"mascot,omitempty"`
	Version int64  `json:"version,omitempty"`
}

// ProfilePatch changes only the fields it carries.
type ProfilePatch struct {
	DisplayName *string `json:"display_name"`
	Bio         *string `json:"bio"`
	// Mascot selects a pre-made avatar; "" clears the avatar entirely.
	Mascot *string `json:"mascot"`
}

func knownMascot(id string) bool {
	for _, m := range Mascots {
		if m == id {
			return true
		}
	}
	return false
}

// cleanText trims, collapses whitespace and rejects control characters
// so a profile renders the same everywhere.
func cleanText(s string, max int, field string, multiline bool) (string, error) {
	s = strings.TrimSpace(s)
	if !utf8.ValidString(s) {
		return "", errors.New(field + " is not valid text")
	}
	for _, r := range s {
		if r == '\n' && multiline {
			continue
		}
		if unicode.IsControl(r) {
			return "", errors.New(field + " must not contain control characters")
		}
	}
	if !multiline {
		s = strings.Join(strings.Fields(s), " ")
	}
	if utf8.RuneCountInString(s) > max {
		return "", errors.New(field + " is too long")
	}
	return s, nil
}

// UpdateProfile applies a patch and returns the public record.
func (s *Service) UpdateProfile(id string, p ProfilePatch) (User, error) {
	var out User
	err := s.db.Update(func(tx *bolt.Tx) error {
		var u User
		if err := kv.GetJSON(tx, kv.BUsers, []byte(id), &u); err != nil {
			if errors.Is(err, kv.ErrNotFound) {
				return errors.New("unknown user")
			}
			return err
		}
		if p.DisplayName != nil {
			v, err := cleanText(*p.DisplayName, maxDisplayName, "display_name", false)
			if err != nil {
				return err
			}
			u.Profile.DisplayName = v
		}
		if p.Bio != nil {
			v, err := cleanText(*p.Bio, maxBio, "bio", true)
			if err != nil {
				return err
			}
			u.Profile.Bio = v
		}
		if p.Mascot != nil {
			switch m := *p.Mascot; {
			case m == "":
				u.Profile.Avatar = Avatar{Version: time.Now().UnixMilli()}
			case knownMascot(m):
				u.Profile.Avatar = Avatar{Kind: AvatarMascot, Mascot: m, Version: time.Now().UnixMilli()}
			default:
				return errors.New("unknown mascot")
			}
		}
		out = u.Public()
		return kv.PutJSON(tx, kv.BUsers, []byte(id), u)
	})
	return out, err
}

// SetUploadedAvatar marks the account as using its uploaded picture.
// The caller has already written the file.
func (s *Service) SetUploadedAvatar(id string) (User, error) {
	var out User
	err := s.db.Update(func(tx *bolt.Tx) error {
		var u User
		if err := kv.GetJSON(tx, kv.BUsers, []byte(id), &u); err != nil {
			if errors.Is(err, kv.ErrNotFound) {
				return errors.New("unknown user")
			}
			return err
		}
		u.Profile.Avatar = Avatar{Kind: AvatarUpload, Version: time.Now().UnixMilli()}
		out = u.Public()
		return kv.PutJSON(tx, kv.BUsers, []byte(id), u)
	})
	return out, err
}
