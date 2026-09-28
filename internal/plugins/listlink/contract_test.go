package listlink

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	contract.Run(t, NewAniList(), []contract.Cap{{
		Name:   contracts.CapListLink,
		Sample: contracts.LinkAuthorizeInput{Platform: "anilist", ClientID: "1", RedirectURI: "http://x/cb", State: "s"},
	}})
}
