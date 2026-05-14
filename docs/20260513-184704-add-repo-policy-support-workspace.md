---
id: 20260513-184704-add-repo-policy-support-workspace
title: Add repo-policy support for `workspace_shared_paths`, starting with `.pi`, so managed `jx work`
status: implementing
created: 2026-05-13
updated: 2026-05-14
currentPhase: 6
externalRef: 
origin: 
---

# Add repo-policy support for `workspace_shared_paths`, starting with `.pi`, so managed `jx work`

## Outcome

Add repo-policy support for `workspace_shared_paths`, starting with `.pi`, so managed `jx work` workspaces can reuse local-only checkout state from the primary checkout through symlinks.

The intended flow is: resolve effective repo policy, build a work-add plan from the primary checkout, identify configured paths that actually exist there, validate those link candidates before creating the workspace, create the jj workspace, then symlink the existing local paths into the new workspace. Missing source paths are skipped, not treated as optional config objects or errors.

The core contract is that linked paths are local-only: a configured path that exists in the primary checkout and will be linked must not be tracked in the selected checkout. For nested paths such as `.foo/bar/baz`, only that configured tail path must be untracked; parent directories may be normal tracked directories. Exact duplicate config entries dedupe after normalization, while parent/child overlaps are invalid config.

Post-create symlink failures do not trigger rollback. Once `jj workspace add` succeeds, the workspace is functional; `jx` should report the setup failure and avoid emitting the shell cd target so shell integration does not move the operator into a partially configured workspace.

## Phases

- [x] 1. Repo-policy config surface and validation
- [x] 2. Work-add planning from the primary checkout
- [x] 3. Tracked-path preflight at the jj boundary
- [x] 4. Workspace creation and post-create setup boundary
- [x] 5. Shared-path symlink application
- [ ] 6. Shell integration, docs, and tests

## Phase Details

### Phase 1: Repo-policy config surface and validation

Add `workspace_shared_paths` to base repo policy and matching repo rules as a list of repo-relative paths. Effective policy should compose in declaration order, normalize paths, dedupe exact duplicates, and reject invalid path shapes: empty paths, absolute paths, `..`, repo escapes, and parent/child overlaps.

This validation belongs in the repo-policy layer because it is independent of any workspace target revision. Keeping overlap rejection here avoids workspace setup having to reason about ambiguous link ownership.

### Phase 2: Work-add planning from the primary checkout

Refactor `jx work add` around an explicit plan that carries the repository identity, primary checkout root, destination workspace root, selected target revision, task metadata intent, and effective shared paths.

The primary checkout is always the symlink source, even when the command runs from an existing managed workspace. During planning, split configured paths into existing link candidates and missing-source skips. Missing paths are not errors and should not require a richer optional-path config shape.

### Phase 3: Tracked-path preflight at the jj boundary

Before `jj workspace add`, validate deterministic failures that would make linking invalid: the destination workspace path already exists, and each existing link candidate is tracked in the selected checkout.

The tracked-path check should test the exact configured path only. For `.foo/bar/baz`, `baz` must be untracked; `.foo` and `.foo/bar` may be tracked directories. This preserves the local-only invariant without overfitting preflight to every possible parent-directory filesystem shape.

### Phase 4: Workspace creation and post-create setup boundary

After preflight succeeds, create the jj workspace and write any managed-workspace metadata through the existing workspace-management boundary. If workspace creation fails, the command stops with no special cleanup because jj did not complete the workspace creation.

Post-create setup failures, including metadata or shared-path symlink failures, should be reported without rolling back the workspace. The design choice is that a created workspace is still usable, and cleanup/retry can be operator-driven rather than hidden behind fragile compensating actions.

### Phase 5: Shared-path symlink application

Apply symlinks only for existing link candidates. For each candidate, create containing directories in the destination workspace, then symlink `workspace/path -> primary/path` as-is. Do not inspect or normalize the source entity type; files, directories, and symlinks are all linked the same way.

The destination side should remain conservative: fail on unexpected existing content rather than overwriting. For nested paths, parent creation is part of this boundary; if a parent is a file or otherwise blocks setup, report that filesystem error clearly.

### Phase 6: Shell integration, docs, and tests

Emit hidden shell cd output only after preflight, workspace creation, metadata writes, and shared-path symlink application all succeed. If symlink setup fails, return an error and do not `pushd`; the workspace remains available for manual repair.

Document `workspace_shared_paths` under repo policy. Tests should use hypothetical repositories/entities only and cover policy composition, path normalization, overlap rejection, missing-source skip behavior, primary-checkout source selection, `.pi` sharing, nested parent creation, tracked-link preflight failure, post-create symlink failure without rollback, and no shell cd output on setup failure.

## Plan Notes

## Summary

Add repo-policy support for `workspace_shared_paths`, starting with `.pi`, so managed `jx work` workspaces can reuse local-only checkout state from the primary checkout through symlinks.

The intended flow is: resolve effective repo policy, build a work-add plan from the primary checkout, identify configured paths that actually exist there, validate those link candidates before creating the workspace, create the jj workspace, then symlink the existing local paths into the new workspace. Missing source paths are skipped, not treated as optional config objects or errors.

The core contract is that linked paths are local-only: a configured path that exists in the primary checkout and will be linked must not be tracked in the selected checkout. For nested paths such as `.foo/bar/baz`, only that configured tail path must be untracked; parent directories may be normal tracked directories. Exact duplicate config entries dedupe after normalization, while parent/child overlaps are invalid config.

Post-create symlink failures do not trigger rollback. Once `jj workspace add` succeeds, the workspace is functional; `jx` should report the setup failure and avoid emitting the shell cd target so shell integration does not move the operator into a partially configured workspace.

## Implementation details

### 1. Repo-policy config surface and validation

Add `workspace_shared_paths` to base repo policy and matching repo rules as a list of repo-relative paths. Effective policy should compose in declaration order, normalize paths, dedupe exact duplicates, and reject invalid path shapes: empty paths, absolute paths, `..`, repo escapes, and parent/child overlaps.

This validation belongs in the repo-policy layer because it is independent of any workspace target revision. Keeping overlap rejection here avoids workspace setup having to reason about ambiguous link ownership.

### 2. Work-add planning from the primary checkout

Refactor `jx work add` around an explicit plan that carries the repository identity, primary checkout root, destination workspace root, selected target revision, task metadata intent, and effective shared paths.

The primary checkout is always the symlink source, even when the command runs from an existing managed workspace. During planning, split configured paths into existing link candidates and missing-source skips. Missing paths are not errors and should not require a richer optional-path config shape.

### 3. Tracked-path preflight at the jj boundary

Before `jj workspace add`, validate deterministic failures that would make linking invalid: the destination workspace path already exists, and each existing link candidate is tracked in the selected checkout.

The tracked-path check should test the exact configured path only. For `.foo/bar/baz`, `baz` must be untracked; `.foo` and `.foo/bar` may be tracked directories. This preserves the local-only invariant without overfitting preflight to every possible parent-directory filesystem shape.

### 4. Workspace creation and post-create setup boundary

After preflight succeeds, create the jj workspace and write any managed-workspace metadata through the existing workspace-management boundary. If workspace creation fails, the command stops with no special cleanup because jj did not complete the workspace creation.

Post-create setup failures, including metadata or shared-path symlink failures, should be reported without rolling back the workspace. The design choice is that a created workspace is still usable, and cleanup/retry can be operator-driven rather than hidden behind fragile compensating actions.

### 5. Shared-path symlink application

Apply symlinks only for existing link candidates. For each candidate, create containing directories in the destination workspace, then symlink `workspace/path -> primary/path` as-is. Do not inspect or normalize the source entity type; files, directories, and symlinks are all linked the same way.

The destination side should remain conservative: fail on unexpected existing content rather than overwriting. For nested paths, parent creation is part of this boundary; if a parent is a file or otherwise blocks setup, report that filesystem error clearly.

### 6. Shell integration, docs, and tests

Emit hidden shell cd output only after preflight, workspace creation, metadata writes, and shared-path symlink application all succeed. If symlink setup fails, return an error and do not `pushd`; the workspace remains available for manual repair.

Document `workspace_shared_paths` under repo policy. Tests should use hypothetical repositories/entities only and cover policy composition, path normalization, overlap rejection, missing-source skip behavior, primary-checkout source selection, `.pi` sharing, nested parent creation, tracked-link preflight failure, post-create symlink failure without rollback, and no shell cd output on setup failure.
