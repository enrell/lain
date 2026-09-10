package gateway

import (
	"encoding/json"
	"net/http"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
)

func decodeLibrary(raw []byte, lib *contracts.Library) error {
	return json.Unmarshal(raw, lib)
}

func (s *Server) handleUsersList(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, 200, s.auth.List())
}

func (s *Server) handleUserCreate(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	var in struct {
		Username string `json:"username"`
		Password string `json:"password"`
		Role     string `json:"role"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	if in.Role == "" {
		in.Role = auth.RoleUser
	}
	u, err := s.auth.Create(in.Username, in.Password, in.Role)
	if err != nil {
		writeErr(w, 400, err.Error())
		return
	}
	writeJSON(w, 201, u)
}

// handleUserPatch applies admin user management: disable, role,
// password reset. At most one semantic per request is too cute;
// accept any subset, apply in a fixed order, report the result.
func (s *Server) handleUserPatch(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	id := r.PathValue("id")
	var in struct {
		Disabled *bool   `json:"disabled"`
		Role     *string `json:"role"`
		Password *string `json:"password"`
	}
	if !s.decode(w, r, &in) {
		return
	}
	if id == v.UserID && in.Disabled != nil && *in.Disabled {
		writeErr(w, 400, "cannot disable yourself")
		return
	}
	if in.Disabled != nil {
		if err := s.auth.SetDisabled(id, *in.Disabled); err != nil {
			writeErr(w, 404, err.Error())
			return
		}
	}
	if in.Role != nil {
		if id == v.UserID && *in.Role != auth.RoleAdmin {
			writeErr(w, 400, "cannot demote yourself")
			return
		}
		if err := s.auth.SetRole(id, *in.Role); err != nil {
			writeErr(w, 400, err.Error())
			return
		}
	}
	if in.Password != nil {
		if err := s.auth.AdminReset(id, *in.Password); err != nil {
			writeErr(w, 400, err.Error())
			return
		}
	}
	u, ok := s.auth.Get(id)
	if !ok {
		writeErr(w, 404, "unknown user")
		return
	}
	writeJSON(w, 200, u)
}
