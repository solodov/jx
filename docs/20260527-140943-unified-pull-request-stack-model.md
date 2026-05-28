---
id: 20260527-140943-unified-pull-request-stack-model
title: Unified Pull Request Stack Model Across Publishing, Selection, Sync, and Rendering
status: done
created: 2026-05-27
updated: 2026-05-28
currentPhase: 
externalRef: 
origin: 
---

# Unified Pull Request Stack Model Across Publishing, Selection, Sync, and Rendering

## Outcome

Build a shared pull-request stack abstraction that becomes the single model for stack-aware PR workflows. Today stack state, GitHub PR lookup, selector hierarchy, PR publishing, stack syncing, and PR body rendering each assemble related data in slightly different ways. The intended direction is to introduce a durable stack snapshot/model plus a command-side manager that merges local `.jx/stack.toml`, jj bookmark facts, and GitHub PR records into one consistent view.

After the change, `jx pr`, `jx open pr -i`, `jx stack`, `jx sync -s`, and generated PR description stack blocks should all operate from the same stack model. This should make stack state maintenance automatic when publishing PRs, preserve merged/disappeared parents, and let interactive PR selection show stack state rather than only live open PR hierarchy.

Key design choice: keep the pure stack model separate from orchestration. The domain model should own tree/component ordering, status derivation, and metadata merge/upsert behavior. A command-side pull request manager should own IO and integration boundaries: reading/writing stack metadata, asking jj for local bookmark facts, and asking GitHub for live PR records.

## Phases

- [x] 1. Establish a canonical pull-request stack snapshot model
- [x] 2. Move tree/component logic into the stack domain layer
- [x] 3. Create a command-side pull request manager for IO and service orchestration
- [x] 4. Make jx pr maintain stack state as part of publishing
- [x] 5. Make jx open pr -i select from the stack snapshot
- [x] 6. Unify stack rendering for jx stack, jx sync -s, and PR descriptions
- [x] 7. Add GitHub refresh support for durable PR numbers

## Phase Details

### Phase 1: Establish a canonical pull-request stack snapshot model

Introduce a shared data shape representing stack nodes independently of how each node was discovered. A `PullRequestStackSnapshot` should contain ordered `PullRequestStackNode`s with branch/base/parent relationships, optional PR details, local availability, current selection, and status flags such as draft/merged/current. This model should be renderer-agnostic and should preserve stored ancestors even when bookmarks disappear locally.

Open questions:
- Whether the model should include closed-but-unmerged PRs distinctly, or continue the current merged/unmerged/draft-only distinction.
- Whether current selection should be branch-first, PR-number-first, or both when local bookmark and stored PR metadata disagree.

### Phase 2: Move tree/component logic into the stack domain layer

Extract hierarchy construction, merge-order sorting, component selection, and missing-ancestor retention into pure domain functions. This replaces duplicated tree logic currently spread across selector rendering, stack metadata rendering, sync stack branch selection, and PR body rendering. The output should support both full-stack views and “component around current branch/PR” views.

The important boundary improvement is that command handlers and renderers should ask for “the stack component for this current branch” or “the ordered selectable PR rows,” not rebuild parent/child maps themselves.

### Phase 3: Create a command-side pull request manager for IO and service orchestration

Add a manager-like abstraction that sits above `CommandServices` and below command handlers. It should load stack metadata from the repo root, gather local PR bookmark candidates from jj, refresh live GitHub PR records, and return canonical snapshots. It should also own write-back decisions such as upserting published PRs and preserving stale-but-useful ancestors.

This manager should not become a second domain layer. Its job is integration: metadata IO, service calls, and converting external facts into the stack snapshot. Keeping this boundary explicit should prevent `jx pr`, `jx open`, `jx stack`, and `jx sync` from each inventing a slightly different GitHub/metadata workflow.

### Phase 4: Make jx pr maintain stack state as part of publishing

After creating or updating a PR, `jx pr` should upsert the resulting PR into stack metadata using the planned head/base relationship. If the new PR belongs to a stack, the manager should refresh the affected component and sync generated stack context across all relevant PR descriptions, including roots. This makes PR body stack context a rendered output of local stack state, not a source of truth.

Risks:
- Publishing a PR against a base branch whose PR is missing or merged needs clear behavior: preserve existing parent metadata when known, otherwise record the base branch without inventing a parent PR.
- Updating multiple PR descriptions after `jx pr` may surprise users if not reflected in output; the command summary should make the effect visible without becoming noisy.

### Phase 5: Make jx open pr -i select from the stack snapshot

Interactive PR opening should show the stack hierarchy and status from the same snapshot used elsewhere. Rows can include merged/draft/current symbols and tree indentation, while still selecting only nodes that have an openable PR URL or number. Stored merged ancestors should be visible when they explain stack context, but likely not selectable if they cannot be opened meaningfully.

Open question: whether `jx open pr -i` should default to the current stack component or include all locally known stack components in the repo. Current-component-first is likely better for focus, with a fallback to all local PR bookmarks when no stack metadata exists.

### Phase 6: Unify stack rendering for jx stack, jx sync -s, and PR descriptions

Move existing stack display and PR body rendering onto the canonical snapshot. `jx stack` should render the stored/live snapshot, `jx sync -s` should choose push branches from the current component, and PR descriptions should render the same tree/status semantics with markdown-safe formatting. This keeps symbols and ordering stable across CLI output and GitHub descriptions.

The main tradeoff is how much GitHub refreshing to do for read-only commands. A fast local-only `jx stack` is useful, but a live-enriched view is more accurate for draft/merged/title changes. The plan should decide whether to make live refresh default, optional, or limited to commands that already require GitHub.

### Phase 7: Add GitHub refresh support for durable PR numbers

To keep disappeared or merged parents accurate, the GitHub boundary likely needs a fetch-by-PR-number operation. Branch-based lookup works for local bookmarks and open PRs, but stored metadata can outlive branches. Fetch-by-number lets the manager refresh title, URL, draft, and merged state without depending on the head branch still existing.

Risk: GitHub API state for closed/unmerged PRs may need careful mapping into the simplified symbols. If the product requirement remains merged/unmerged/draft only, closed-unmerged PRs should be treated as unmerged unless we explicitly add a separate blocked/closed state later.

## Plan Notes

## Summary

Build a shared pull-request stack abstraction that becomes the single model for stack-aware PR workflows. Today stack state, GitHub PR lookup, selector hierarchy, PR publishing, stack syncing, and PR body rendering each assemble related data in slightly different ways. The intended direction is to introduce a durable stack snapshot/model plus a command-side manager that merges local `.jx/stack.toml`, jj bookmark facts, and GitHub PR records into one consistent view.

After the change, `jx pr`, `jx open pr -i`, `jx stack`, `jx sync -s`, and generated PR description stack blocks should all operate from the same stack model. This should make stack state maintenance automatic when publishing PRs, preserve merged/disappeared parents, and let interactive PR selection show stack state rather than only live open PR hierarchy.

Key design choice: keep the pure stack model separate from orchestration. The domain model should own tree/component ordering, status derivation, and metadata merge/upsert behavior. A command-side pull request manager should own IO and integration boundaries: reading/writing stack metadata, asking jj for local bookmark facts, and asking GitHub for live PR records.

## Implementation details

1. **Establish a canonical pull-request stack snapshot model**

   Introduce a shared data shape representing stack nodes independently of how each node was discovered. A `PullRequestStackSnapshot` should contain ordered `PullRequestStackNode`s with branch/base/parent relationships, optional PR details, local availability, current selection, and status flags such as draft/merged/current. This model should be renderer-agnostic and should preserve stored ancestors even when bookmarks disappear locally.

   Open questions:
   - Whether the model should include closed-but-unmerged PRs distinctly, or continue the current merged/unmerged/draft-only distinction.
   - Whether current selection should be branch-first, PR-number-first, or both when local bookmark and stored PR metadata disagree.

2. **Move tree/component logic into the stack domain layer**

   Extract hierarchy construction, merge-order sorting, component selection, and missing-ancestor retention into pure domain functions. This replaces duplicated tree logic currently spread across selector rendering, stack metadata rendering, sync stack branch selection, and PR body rendering. The output should support both full-stack views and “component around current branch/PR” views.

   The important boundary improvement is that command handlers and renderers should ask for “the stack component for this current branch” or “the ordered selectable PR rows,” not rebuild parent/child maps themselves.

3. **Create a command-side pull request manager for IO and service orchestration**

   Add a manager-like abstraction that sits above `CommandServices` and below command handlers. It should load stack metadata from the repo root, gather local PR bookmark candidates from jj, refresh live GitHub PR records, and return canonical snapshots. It should also own write-back decisions such as upserting published PRs and preserving stale-but-useful ancestors.

   This manager should not become a second domain layer. Its job is integration: metadata IO, service calls, and converting external facts into the stack snapshot. Keeping this boundary explicit should prevent `jx pr`, `jx open`, `jx stack`, and `jx sync` from each inventing a slightly different GitHub/metadata workflow.

4. **Make `jx pr` maintain stack state as part of publishing**

   After creating or updating a PR, `jx pr` should upsert the resulting PR into stack metadata using the planned head/base relationship. If the new PR belongs to a stack, the manager should refresh the affected component and sync generated stack context across all relevant PR descriptions, including roots. This makes PR body stack context a rendered output of local stack state, not a source of truth.

   Risks:
   - Publishing a PR against a base branch whose PR is missing or merged needs clear behavior: preserve existing parent metadata when known, otherwise record the base branch without inventing a parent PR.
   - Updating multiple PR descriptions after `jx pr` may surprise users if not reflected in output; the command summary should make the effect visible without becoming noisy.

5. **Make `jx open pr -i` select from the stack snapshot**

   Interactive PR opening should show the stack hierarchy and status from the same snapshot used elsewhere. Rows can include merged/draft/current symbols and tree indentation, while still selecting only nodes that have an openable PR URL or number. Stored merged ancestors should be visible when they explain stack context, but likely not selectable if they cannot be opened meaningfully.

   Open question: whether `jx open pr -i` should default to the current stack component or include all locally known stack components in the repo. Current-component-first is likely better for focus, with a fallback to all local PR bookmarks when no stack metadata exists.

6. **Unify stack rendering for `jx stack`, `jx sync -s`, and PR descriptions**

   Move existing stack display and PR body rendering onto the canonical snapshot. `jx stack` should render the stored/live snapshot, `jx sync -s` should choose push branches from the current component, and PR descriptions should render the same tree/status semantics with markdown-safe formatting. This keeps symbols and ordering stable across CLI output and GitHub descriptions.

   The main tradeoff is how much GitHub refreshing to do for read-only commands. A fast local-only `jx stack` is useful, but a live-enriched view is more accurate for draft/merged/title changes. The plan should decide whether to make live refresh default, optional, or limited to commands that already require GitHub.

7. **Add GitHub refresh support for durable PR numbers**

   To keep disappeared or merged parents accurate, the GitHub boundary likely needs a fetch-by-PR-number operation. Branch-based lookup works for local bookmarks and open PRs, but stored metadata can outlive branches. Fetch-by-number lets the manager refresh title, URL, draft, and merged state without depending on the head branch still existing.

   Risk: GitHub API state for closed/unmerged PRs may need careful mapping into the simplified symbols. If the product requirement remains merged/unmerged/draft only, closed-unmerged PRs should be treated as unmerged unless we explicitly add a separate blocked/closed state later.
