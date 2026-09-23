package playback


import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/testutil/contract"
)

func TestProviderContract(t *testing.T) {
	contract.Run(t, Planner{}, []contract.Cap{{
		Name: contracts.CapPlaybackPlan,
		Sample: PlanInput{
			Request:  contracts.PlanRequest{ItemID: "i1", Client: "browser"},
			FilePath: "/media/Show - S01E02.mkv",
		},
	}})
}
