# Review management

`jx review` is a GitHub review inbox for pull requests that currently involve the authenticated user. It fetches open PRs from GitHub, applies repository review policy, and renders compact check, review, lag, title, label, and author columns.

Use repository filters to narrow the inbox by configured layout key or provider/owner/repo glob:

```sh
jx review api-*
jx review example-owner/example-repo
jx review --format json
jx review --cached --format json
jx review -i --refresh-seconds 60
```

The interactive dashboard is for the normal live inbox only. JSON output is non-interactive so external selectors can consume stable provider data. `jx review --cached` renders the latest locally stored review inbox and PR snapshots without contacting GitHub, so it is fast but can be stale.

## Local dismissal

Review dismissal is local. `jx` does not submit, dismiss, approve, or otherwise mutate GitHub reviews to clean the inbox.

```sh
jx review dismiss 123
jx review dismiss api-alpha#123
jx review dismiss example-owner/api-alpha#123
jx review dismiss https://github.com/example-owner/api-alpha/pull/123

jx review dismissed
jx review history api-alpha#123
jx review undismiss api-alpha#123
```

`jx review history` reads the local store without mutating GitHub and shows the derived PR history next to local `dismiss`/`undismiss` actions for debugging visibility decisions. Event times render in the operator's local time zone; JSON also includes `changedAtUnix` for exact comparisons.

Dismissal selectors are suffix-style and component-aware:

- `123` matches only when that PR number identifies one review row.
- `repo#123` matches a repository name suffix.
- `owner/repo#123` matches the full GitHub repository.
- GitHub pull-request URLs normalize to the same owner/repo selector.

Manual dismissal hides a PR until it needs attention again. New commits, a fresh direct review request, or a meaningful author response resurface non-draft PRs. Draft dismissals ignore head churn and resurface when the PR becomes ready for review, directly requests the viewer, mentions the viewer, or receives a meaningful author response.

`jx review undismiss` records a local override so the PR returns to the normal inbox even if the current snapshot would otherwise be auto-hidden as already handled.

## Automatic hiding

`jx review` computes some hidden states from the current PR snapshot instead of storing them as local dismissals:

- `non_default_branch`: the PR targets a branch other than the base repository's default branch.
- `approved`: the viewer already approved the PR.
- `commented`: the viewer commented or requested changes and there is no newer author response requiring attention.
- `draft`: the PR is still draft and has no explicit attention signal.

`jx review dismissed` shows both manual/action-backed dismissals and these computed hidden rows with synthetic labels such as `jx:dismissed:manual`, `jx:dismissed:non_default_branch`, `jx:dismissed:approved`, `jx:dismissed:commented`, or `jx:dismissed:draft`.

Author comments matching `repo.review.ignored_author_response_comments` do not count as meaningful responses for resurfacing. This is intended for command-only comments such as automation merge commands.

## Local state files

Review visibility decisions use the shared pull-request store plus current GitHub snapshots:

- `$XDG_STATE_HOME/jx/pull-request-store.sqlite`
- fallback: `~/.local/state/jx/pull-request-store.sqlite`

The store is personal, single-operator state. It keeps normalized PR snapshots, derived PR history, local review actions such as `dismiss` and `undismiss`, and the latest viewer-scoped review inbox candidate set used by `jx review --cached`. Timestamp columns ending in `_at_unix` store UTC Unix seconds.

Dismissal audit events append to:

- `$XDG_STATE_HOME/jx/review-dismissals.log`
- fallback: `~/.local/state/jx/review-dismissals.log`

The audit log is JSONL and write-only for decision-making: current visibility is not read from the log. A legacy `review-dismissals.log.jsonl` file is renamed to `review-dismissals.log` on the next audit write.

`review-dismissals.toml` is no longer used. Existing TOML files are ignored by current review visibility decisions.
