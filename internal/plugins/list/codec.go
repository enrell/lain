package list

import (
	"encoding/json"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

func decodeEntry(raw []byte, e *contracts.ListEntry) error {
	return json.Unmarshal(raw, e)
}

func decodeAccount(raw []byte, a *contracts.LinkedAccount) error {
	return json.Unmarshal(raw, a)
}

func nowUnix() int64 { return time.Now().Unix() }
