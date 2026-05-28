---
id: 20260528-large-repo-clone-optimizations
title: Large-repository clone optimizations for jj-backed checkouts
status: blocked
created: 2026-05-28
updated: 2026-05-28
currentPhase: 
externalRef: https://github.com/jj-vcs/jj/issues/8920
origin: 
---

# Large-repository clone optimizations for jj-backed checkouts

## Outcome

Explore faster clone strategies for large repositories where local history and
historical blobs are usually unnecessary. The desired end state is a `jx clone`
path that optimizes the common developer workflow without creating jj-backed
repositories that fail later during normal diff, checkout, rebase, sync, or
workspace operations.

## Findings

- Blobless Git partial clone (`--filter=blob:none`) is the best fit for
  long-lived developer checkouts because it keeps commit and tree history while
  lazy-fetching file blobs on demand.
- Mainline jj does not currently support Git partial clones reliably. A manually
  created blobless Git clone can fail in jj when jj reads a missing promisor
  object through its Git backend instead of Git's lazy-fetch path.
- `jj git clone` exposes shallow clone depth, but jj documents shallow clones as
  only partially supported. Deepening or fully unshallowing a repository is not
  currently supported and can cause issues.
- `jx` should not create blobless jj-backed repositories until upstream jj can
  hydrate missing promisor objects safely during normal operations.

## Future direction

Track upstream jj partial-clone support and revisit clone optimization once jj
can operate on missing promisor objects without surfacing backend object-not-found
errors. At that point, prefer a blobless strategy for large long-lived repos over
shallow clone. Shallow clone may still be useful later as an explicit
throwaway/CI-oriented mode, but it should not be the default developer checkout
strategy.

Potential `jx` design once upstream support is ready:

- Add an explicit clone strategy to layout config, defaulting to the current full
  clone behavior.
- Allow source- or rule-level overrides so only known large repositories opt into
  optimized clone behavior.
- Keep command output clear about the selected clone strategy and any jj/Git
  limitations that remain.

## References

- https://github.com/jj-vcs/jj/issues/8920
- https://github.com/jj-vcs/jj/pull/6451
- https://github.com/jj-vcs/jj/issues/6690
- https://github.com/GitoxideLabs/gitoxide/issues/1046
- https://docs.jj-vcs.dev/latest/git-compatibility/
