package userstate

import (
	"encoding/json"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

func decodeProgress(raw []byte, p *contracts.Progress) error {
	return json.Unmarshal(raw, p)
}

func nowUnix() int64 { return time.Now().Unix() }
