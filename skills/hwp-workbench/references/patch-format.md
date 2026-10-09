# Patch format and error hints

## Full patch format

```json
{
  "schemaVersion": 1,
  "baseSha256": "SHA-256 from the current read or inspect result",
  "operations": [
    {
      "op": "set_text",
      "nodeId": "Contents/section0.xml#text:6",
      "expectedText": "exact original text",
      "text": "replacement text"
    }
  ]
}
```

Only `set_text` is supported. Each operation requires exactly these four keys.
HWP IDs resemble `BodyText/Section0#record:1:text:0`. Short references printed by
`read`, such as `s0:t6`, resolve against the current source. Preview patches and
published proposals contain full native IDs. Never copy an ID across revisions.

`baseSha256` must equal the source's SHA-256. `expectedText` must match exactly,
including Unicode and spaces. Patches contain 1–1000 unique targets; a short
reference and full ID for the same node are duplicates. Validation is complete
and all-or-nothing. No partial patch is saved after a failed operation.

`text` is a Unicode string of at most 200,000 characters. Native controls, tabs,
newlines and invalid Unicode are refused. HWP and HWPX allow variable-length edits
inside existing plain-text runs; an HWP edit that would touch a native control is
refused with `UNSUPPORTED_ELEMENT`, and Hancom reflow is unverified.
Neither format supports adding paragraphs, styles or table structure through this patch.

## Preview, plan, apply and propose

`replace`, `fill` and `set` return an envelope with `patch`, `changes`, `warnings`
and operation-specific reports (`skipped`, or `filled`/`unresolved`). Review all
reports. Empty previews cannot be applied or proposed as edits.

`plan --patch FILE`, `apply --patch FILE` and `propose --patch FILE` accept either
a patch object or the whole saved preview envelope. They use its `patch` and run
all hash, expected-text, unique-target and native-format checks. Review the
preview, then pass that same file without rerunning the edit:

```bash
"{{HWP_WORKBENCH}}" plan form.hwpx --patch work/preview.json
"{{HWP_WORKBENCH}}" apply form.hwpx --patch work/preview.json \
  -o form-reviewed.hwpx --acknowledge-risks
```

`propose --patch FILE --title TITLE [--author AUTHOR] [--notes NOTES]` validates the
patch and writes a new proposal in `.hwp-workbench/proposals/` beside the document.
The author defaults to `agent`; the result has `proposal` (workspace-relative path)
and `changes`. A proposal is limited to 2 MiB and 1000 operations. Publication
never overwrites.

`session PATH` is read-only. It returns `linked`, `session` (or null) and at most
200 proposal summaries with `file`, `title`, `author`, `createdAt`, operation count
and `status`. Status can be `pending`, `stale`, `unreadable`, `accepted`, `declined`,
or `partial`. It does not enable Agent link or accept a proposal.

Session: `.hwp-workbench/<document file name>.session.json`.
Proposal: `.hwp-workbench/proposals/<document file name>.<UTC yyyymmddThhmmssZ>-<6 hex>.json`.
Outcome: `<proposal file stem>.outcome.json`, next to the proposal.
Only the desktop writes sessions (while Agent link is enabled) and outcomes.
Outcome JSON contains `schemaVersion: 1`, `status`, `acceptedNodeIds`,
`declinedNodeIds`, and `decidedAt`. Read it as data; never alter it to imply approval.

## Error hints

Errors are JSON on stderr; exit code is 2. The error has `code`, `message`, `hint`
and `retryable`. Retryable means retry **after correction**, not repeat unchanged.

| Code | Action |
|---|---|
| `STALE_DOCUMENT`, `STALE_TEXT` | Read the current source and rebuild the patch; keep exact hash and expected text. |
| `NODE_NOT_FOUND` | Get an ID or short reference from the current read/search. |
| `OUTPUT_EXISTS` | Choose a new output or proposal name; never delete a user's file to retry. |
| `OUTPUT_FORMAT` | Keep the detected source format's native extension. |
| `RISK_ACK_REQUIRED` | Review the diff and warnings before acknowledging risks. |
| `BAD_PATCH`, `CONTROL_TEXT` | Correct the patch keys, operation count or plain-text value. |
| `BAD_ARGUMENTS` | Check subcommand help; use either positional search or `--query`, and either `--field` or `--json`. |
| `REGEX_LIMIT`, `REGEX_UNSUPPORTED`, `BAD_REGEX` | Use a literal search or bounded fixed-width regex. |
| `OUTSIDE_WORKSPACE`, `UNSAFE_PATH`, `READ_FAILED` | Use regular files and existing parents inside the approved root, without symlinks. |
| `LIMIT` | Reduce request/file size to the stated bound. |
| `SAVE_BLOCKED`, protected/unsupported format errors | Keep the original; do not bypass protection. |
| `VERIFY_FAILED`, malformed native container/XML errors | Keep the original and report the failure; do not publish an unverified output. |

No successful command certifies Hancom layout or complete native edit support.
