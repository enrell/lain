package catalog

import (
	"bytes"
	"encoding/json"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

func marshalItem(it contracts.CatalogItem) ([]byte, error) {
	return json.Marshal(it)
}

func unmarshalItem(raw []byte, it *contracts.CatalogItem) error {
	return json.Unmarshal(raw, it)
}

func hasPrefix(k, prefix []byte) bool { return bytes.HasPrefix(k, prefix) }

func nowUnix() int64 { return time.Now().Unix() }
