# Component protocol fixtures (NDJSON)

Each `*.ndjson` file is one recorded session of the component wire
protocol (D-074, `docs/CONTRACTS.md` "Component mode"). One line is one
frame:

```json
{"from": "component" | "host", "note": "optional", "frame": { ... }}
```

- The first frame is the component's `hello`
  (`{op:"hello", provider, protocol:1, capabilities}`).
- Host frames are requests: `{seq, cap?, op?, input}`; `op:"health"`
  with no `cap` is the transport-level liveness op.
- Each host frame is followed by the component's answer: `{seq,
  output}` or `{seq, error:{code, msg}}`.

Values are compared as JSON. The string `"$string"` matches any string,
`"$any"` any value; everything else must be equal. Error codes are
always exact; messages are exact only where every implementation must
agree (`cap required`).

Who replays them:

| Side | Test | What it proves |
|---|---|---|
| Go component (`component.Serve`) | `internal/component/conformance_test.go` | the recorded answers are what Go serves today |
| Rust host (`lain-component`) | `crates/lain-component/tests/fixtures.rs` | the Rust host sends these frames and reads these answers |
| Rust component (`lain_component::serve`) | `crates/lain-component/tests/fixtures.rs` | a Rust component answers like the Go one |

`identify-generic.ndjson` uses the built-in `lain-identify-generic`
provider, which `lain plugin-run --id lain-identify-generic` serves.
