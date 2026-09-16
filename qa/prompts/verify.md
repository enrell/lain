# Re-test brief — {{SEED}} (round {{ROUND}})

You filed these findings earlier in this same conversation. The remediation
engineer has since changed the code. Your job now is narrow and honest:
**re-test your own findings in the browser and say what happened.**

Nothing else is in scope. Do not audit new ground, do not add findings, do not
fix code. The target is a fresh build of the same repository; the repository
revision is now `{{REVISION}}`.

## Target

- Base URL: {{BASE_URL}}
- Administrator account: username `{{ADMIN_USER}}` password `{{ADMIN_PASSWORD}}`
- Member account: username `{{MEMBER_USER}}` password `{{MEMBER_PASSWORD}}`
- Your browser state dir (already running): `{{BROWSER_STATE}}`
- Your findings for this round:

{{FINDINGS_BLOCK}}

## What the engineer claims it did

{{FIX_BLOCK}}

## Protocol

For every finding listed above, in order:

1. Re-run the original repro steps literally. If the environment changed (new
   seeded content, different ids), adapt the minimum and say what you adapted.
2. Take the same kind of evidence you took the first time — snapshot, screenshot,
   console, computed styles — and put it in `evidence`.
3. Decide:

| verdict | when |
| --- | --- |
| `fixed` | the behaviour you asked for in `expected` now happens |
| `partially-fixed` | better, but the defect still shows in some case |
| `not-fixed` | still reproduces exactly as before |
| `cannot-verify` | you could not reach the condition; explain the blocker |
| `not-a-defect` | on re-reading your own evidence you were wrong |
| `wont-fix` | the engineer marked it as needing a human decision and did not touch it |

`fixed` is a promise you are making to a human: only make it when you watched
the correct behaviour happen.

Also check for collateral damage: if the change that fixed your finding broke a
neighbouring flow you know well, report it under `remaining` with a one-line
repro — the fixer will see it.

## Output

Write `{{VERDICT_JSON}}` exactly in this shape, then print the two-sentence
verdict summary and stop:

```json
{{VERDICT_EXAMPLE}}
```
