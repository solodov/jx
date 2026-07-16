use super::*;
use crate::github::{PullRequestStatusRecord, PullRequestTimelineEventKind};
use rusqlite::{params, OptionalExtension as _};

pub const PULL_REQUEST_STORE_FILE: &str = "pull-request-store.sqlite";
pub const PULL_REQUEST_STORE_SCHEMA_VERSION: i64 = 1;
pub const PULL_REQUEST_SNAPSHOT_SCHEMA_VERSION: i64 = 1;

const CREATE_PULL_REQUEST_STORE_SCHEMA: &str = r#"
-- All *_at_unix columns store UTC Unix timestamps in seconds.

-- Tracks applied storage migrations.
CREATE TABLE IF NOT EXISTS schema_migrations (
  -- Monotonic migration version.
  version INTEGER PRIMARY KEY,

  -- UTC Unix timestamp in seconds when the migration was applied.
  applied_at_unix INTEGER NOT NULL
);

-- One GitHub repository known to the local store.
CREATE TABLE IF NOT EXISTS repositories (
  -- Local stable repository id.
  id INTEGER PRIMARY KEY,

  -- Git host. Defaults to github.com; kept for future GitHub Enterprise support.
  host TEXT NOT NULL DEFAULT 'github.com',

  -- Repository owner/org login.
  owner TEXT NOT NULL,

  -- Repository name.
  name TEXT NOT NULL,

  UNIQUE(host, owner, name)
);

-- One pull request known to the local store.
CREATE TABLE IF NOT EXISTS pull_requests (
  -- Local stable PR id.
  id INTEGER PRIMARY KEY,

  -- Owning repository.
  repository_id INTEGER NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,

  -- Pull request number within the repository.
  number INTEGER NOT NULL,

  -- When jx first saw this PR, as a UTC Unix timestamp in seconds.
  first_seen_at_unix INTEGER NOT NULL,

  -- When jx last refreshed/saw this PR, as a UTC Unix timestamp in seconds.
  last_seen_at_unix INTEGER NOT NULL,

  UNIQUE(repository_id, number)
);

-- Raw normalized pull request state observed from GitHub.
-- This is the source data used to rebuild pull_request_history.
CREATE TABLE IF NOT EXISTS pull_request_snapshots (
  -- Local snapshot id.
  id INTEGER PRIMARY KEY,

  -- Pull request this snapshot belongs to.
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,

  -- When jx observed this snapshot, as a UTC Unix timestamp in seconds.
  observed_at_unix INTEGER NOT NULL,

  -- Version of jx's normalized snapshot JSON shape.
  schema_version INTEGER NOT NULL,

  -- Refresh/dedup guard from GitHub's updatedAt timestamp.
  -- This optional value can be read from a cheap summary query before deep fetch;
  -- if the latest snapshot has the same value, jx can reuse local snapshot/history.
  github_updated_at_unix INTEGER,

  -- Refresh/dedup guard over the full normalized payload.
  -- After deep fetch, this hash avoids duplicate snapshots and unnecessary history rebuilds.
  payload_hash TEXT NOT NULL,

  -- Full normalized PR data needed by stack/review renderers and history extraction.
  payload_json TEXT NOT NULL
);

-- Semantic pull request timeline derived from snapshots.
-- This compact history is what review/stack decision engines normally read.
CREATE TABLE IF NOT EXISTS pull_request_history (
  -- Local history row id.
  id INTEGER PRIMARY KEY,

  -- Pull request this history event belongs to.
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,

  -- Snapshot that produced this history event.
  -- If extraction logic changes, rows can be deleted/rebuilt from snapshots.
  snapshot_id INTEGER NOT NULL REFERENCES pull_request_snapshots(id) ON DELETE CASCADE,

  -- Semantic event kind, e.g. head_changed, review_state_changed, author_response.
  kind TEXT NOT NULL,

  -- When the PR data effectively changed, as a UTC Unix timestamp in seconds.
  -- Prefer GitHub event time when available; otherwise use snapshot observed time.
  changed_at_unix INTEGER NOT NULL,

  -- Previous value for change events, encoded as JSON.
  old_json TEXT,

  -- New/current value for this event, encoded as JSON.
  new_json TEXT,

  -- Additional structured event context, encoded as JSON.
  details_json TEXT NOT NULL DEFAULT '{}'
);

-- Local operator actions that affect decision policy.
-- Actions are single-operator because this is a personal local state store.
CREATE TABLE IF NOT EXISTS pull_request_actions (
  -- Local action row id.
  id INTEGER PRIMARY KEY,

  -- Pull request this action applies to.
  pr_id INTEGER NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,

  -- Action name, e.g. dismiss or undismiss.
  action TEXT NOT NULL,

  -- Origin of the action, e.g. manual or migration.
  source TEXT NOT NULL,

  -- Short action reason, e.g. manual, approved, commented, draft.
  reason TEXT,

  -- When the local action occurred, as a UTC Unix timestamp in seconds.
  changed_at_unix INTEGER NOT NULL,

  -- Additional structured action context, encoded as JSON.
  details_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX IF NOT EXISTS idx_snapshots_pr_latest
  ON pull_request_snapshots(pr_id, id DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_snapshots_pr_payload
  ON pull_request_snapshots(pr_id, payload_hash);

CREATE INDEX IF NOT EXISTS idx_snapshots_pr_github_updated
  ON pull_request_snapshots(pr_id, github_updated_at_unix DESC);

CREATE INDEX IF NOT EXISTS idx_history_pr_time
  ON pull_request_history(pr_id, changed_at_unix DESC);

CREATE INDEX IF NOT EXISTS idx_history_pr_kind_time
  ON pull_request_history(pr_id, kind, changed_at_unix DESC);

CREATE INDEX IF NOT EXISTS idx_history_snapshot
  ON pull_request_history(snapshot_id);

CREATE INDEX IF NOT EXISTS idx_actions_pr_time
  ON pull_request_actions(pr_id, changed_at_unix DESC);
"#;

pub struct PullRequestStore {
    path: PathBuf,
    connection: rusqlite::Connection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullRequestWithHistory {
    pub status: PullRequestStatusRecord,
    pub history: Vec<PullRequestHistoryRecord>,
    pub actions: Vec<PullRequestActionRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PullRequestActionDismissal {
    pub repository: GitHubRepository,
    pub number: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullRequestHistoryRecord {
    pub kind: String,
    pub changed_at_unix: i64,
    pub old_json: Option<serde_json::Value>,
    pub new_json: Option<serde_json::Value>,
    pub details_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PullRequestActionRecord {
    pub action: String,
    pub source: String,
    pub reason: Option<String>,
    pub changed_at_unix: i64,
    pub details_json: serde_json::Value,
}

struct StoredPullRequestSnapshot {
    pr_id: i64,
    status: PullRequestStatusRecord,
}

struct PullRequestHistoryEvent {
    kind: &'static str,
    changed_at_unix: i64,
    old_json: Option<serde_json::Value>,
    new_json: Option<serde_json::Value>,
    details_json: serde_json::Value,
}

impl PullRequestStore {
    /// Opens the operator's local pull-request store and applies pending schema migrations.
    pub fn open(environment: &RuntimeEnvironment) -> Result<Self, RepositoryError> {
        let path = pull_request_store_file(environment)?;
        Self::open_at(path)
    }

    /// Opens a pull-request store at a specific path, mainly for tests and future migrations.
    pub fn open_at(path: impl Into<PathBuf>) -> Result<Self, RepositoryError> {
        let path = path.into();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|source| {
                RepositoryError::PullRequestStoreWrite {
                    file: parent.to_path_buf(),
                    source,
                }
            })?;
        }
        let connection = rusqlite::Connection::open(&path).map_err(|source| {
            RepositoryError::PullRequestStoreOpen {
                file: path.clone(),
                source,
            }
        })?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(|source| RepositoryError::PullRequestStoreMigration {
                file: path.clone(),
                source,
            })?;
        migrate_pull_request_store(&connection, &path)?;
        Ok(Self { path, connection })
    }

    /// Filesystem path for this local store.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Records fresh pull-request snapshots from the GitHub boundary.
    pub fn record_pull_request_snapshots(
        &self,
        repository: &GitHubRepository,
        statuses: &[PullRequestStatusRecord],
    ) -> Result<(), RepositoryError> {
        let repository_id = self.upsert_repository(repository)?;
        let observed_at_unix = chrono::Utc::now().timestamp();
        for status in statuses {
            let pr_id = self.upsert_pull_request(repository_id, status.number, observed_at_unix)?;
            self.insert_pull_request_snapshot(repository, pr_id, status, observed_at_unix)?;
        }
        Ok(())
    }

    /// Loads the latest stored pull-request snapshots in the requested-number order.
    pub fn latest_pull_request_snapshots(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestStatusRecord>, RepositoryError> {
        let mut statuses = Vec::new();
        for number in unique_numbers(numbers) {
            if let Some(snapshot) = self.latest_pull_request_snapshot(repository, number)? {
                statuses.push(snapshot.status);
            }
        }
        Ok(statuses)
    }

    /// Loads current PR snapshots with semantic history and local actions.
    pub fn latest_pull_requests_with_history(
        &self,
        repository: &GitHubRepository,
        numbers: &[u64],
    ) -> Result<Vec<PullRequestWithHistory>, RepositoryError> {
        let mut pull_requests = Vec::new();
        for number in unique_numbers(numbers) {
            let Some(snapshot) = self.latest_pull_request_snapshot(repository, number)? else {
                continue;
            };
            pull_requests.push(PullRequestWithHistory {
                history: self.pull_request_history(snapshot.pr_id)?,
                actions: self.pull_request_actions(snapshot.pr_id)?,
                status: snapshot.status,
            });
        }
        Ok(pull_requests)
    }

    /// Lists PRs whose latest local visibility action is a dismissal.
    pub fn action_dismissed_pull_requests(
        &self,
    ) -> Result<Vec<PullRequestActionDismissal>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT repositories.owner, repositories.name, pull_requests.number
                 FROM pull_requests
                 JOIN repositories ON repositories.id = pull_requests.repository_id
                 JOIN pull_request_actions actions ON actions.pr_id = pull_requests.id
                 WHERE actions.id = (
                     SELECT latest.id
                     FROM pull_request_actions latest
                     WHERE latest.pr_id = pull_requests.id
                       AND latest.action IN ('dismiss', 'undismiss')
                     ORDER BY latest.changed_at_unix DESC, latest.id DESC
                     LIMIT 1
                 )
                   AND actions.action = 'dismiss'
                 ORDER BY repositories.owner, repositories.name, pull_requests.number",
            )
            .map_err(|source| self.query_error(source))?;
        let rows = statement
            .query_map([], |row| {
                Ok(PullRequestActionDismissal {
                    repository: GitHubRepository {
                        owner: row.get(0)?,
                        name: row.get(1)?,
                    },
                    number: row.get::<_, i64>(2)? as u64,
                })
            })
            .map_err(|source| self.query_error(source))?;
        let mut dismissals = Vec::new();
        for row in rows {
            dismissals.push(row.map_err(|source| self.query_error(source))?);
        }
        Ok(dismissals)
    }

    /// Records a local operator action that can affect future PR decisions.
    pub fn record_pull_request_action(
        &self,
        repository: &GitHubRepository,
        number: u64,
        action: &str,
        source: &str,
        reason: Option<&str>,
        details_json: serde_json::Value,
    ) -> Result<(), RepositoryError> {
        let changed_at_unix = chrono::Utc::now().timestamp();
        let repository_id = self.upsert_repository(repository)?;
        let pr_id = self.upsert_pull_request(repository_id, number, changed_at_unix)?;
        let details_json =
            serde_json::to_string(&details_json).expect("action details JSON serializes");
        self.connection
            .execute(
                "INSERT INTO pull_request_actions
                 (pr_id, action, source, reason, changed_at_unix, details_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![pr_id, action, source, reason, changed_at_unix, details_json],
            )
            .map_err(|source| self.query_error(source))?;
        Ok(())
    }

    fn upsert_repository(&self, repository: &GitHubRepository) -> Result<i64, RepositoryError> {
        self.connection
            .execute(
                "INSERT OR IGNORE INTO repositories (host, owner, name) VALUES ('github.com', ?1, ?2)",
                params![&repository.owner, &repository.name],
            )
            .map_err(|source| self.query_error(source))?;
        self.connection
            .query_row(
                "SELECT id FROM repositories WHERE host = 'github.com' AND owner = ?1 AND name = ?2",
                params![&repository.owner, &repository.name],
                |row| row.get(0),
            )
            .map_err(|source| self.query_error(source))
    }

    fn upsert_pull_request(
        &self,
        repository_id: i64,
        number: u64,
        seen_at_unix: i64,
    ) -> Result<i64, RepositoryError> {
        let number = number as i64;
        self.connection
            .execute(
                "INSERT INTO pull_requests (repository_id, number, first_seen_at_unix, last_seen_at_unix)
                 VALUES (?1, ?2, ?3, ?3)
                 ON CONFLICT(repository_id, number) DO UPDATE SET last_seen_at_unix = excluded.last_seen_at_unix",
                params![repository_id, number, seen_at_unix],
            )
            .map_err(|source| self.query_error(source))?;
        self.connection
            .query_row(
                "SELECT id FROM pull_requests WHERE repository_id = ?1 AND number = ?2",
                params![repository_id, number],
                |row| row.get(0),
            )
            .map_err(|source| self.query_error(source))
    }

    fn insert_pull_request_snapshot(
        &self,
        repository: &GitHubRepository,
        pr_id: i64,
        status: &PullRequestStatusRecord,
        observed_at_unix: i64,
    ) -> Result<i64, RepositoryError> {
        let previous = self.latest_pull_request_snapshot_for_pr(pr_id)?;
        let payload_json = serde_json::to_string(status).map_err(|source| {
            RepositoryError::PullRequestSnapshotEncode {
                repository: repository.slug(),
                number: status.number,
                source,
            }
        })?;
        let payload_hash = stable_payload_hash(&payload_json);
        let inserted = self
            .connection
            .execute(
                "INSERT OR IGNORE INTO pull_request_snapshots
                 (pr_id, observed_at_unix, schema_version, github_updated_at_unix, payload_hash, payload_json)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
                params![
                    pr_id,
                    observed_at_unix,
                    PULL_REQUEST_SNAPSHOT_SCHEMA_VERSION,
                    payload_hash,
                    payload_json
                ],
            )
            .map_err(|source| self.query_error(source))?;
        let snapshot_id = self
            .connection
            .query_row(
                "SELECT id FROM pull_request_snapshots WHERE pr_id = ?1 AND payload_hash = ?2",
                params![pr_id, payload_hash],
                |row| row.get(0),
            )
            .map_err(|source| self.query_error(source))?;
        if inserted > 0 {
            self.record_pull_request_history(
                pr_id,
                snapshot_id,
                previous.as_ref().map(|snapshot| &snapshot.status),
                status,
                observed_at_unix,
            )?;
        }
        Ok(snapshot_id)
    }

    fn record_pull_request_history(
        &self,
        pr_id: i64,
        snapshot_id: i64,
        previous: Option<&PullRequestStatusRecord>,
        current: &PullRequestStatusRecord,
        observed_at_unix: i64,
    ) -> Result<(), RepositoryError> {
        for event in pull_request_history_events(previous, current, observed_at_unix) {
            let old_json = event
                .old_json
                .as_ref()
                .map(|value| serde_json::to_string(value).expect("history JSON serializes"));
            let new_json = event
                .new_json
                .as_ref()
                .map(|value| serde_json::to_string(value).expect("history JSON serializes"));
            let details_json = serde_json::to_string(&event.details_json)
                .expect("history details JSON serializes");
            self.connection
                .execute(
                    "INSERT INTO pull_request_history
                     (pr_id, snapshot_id, kind, changed_at_unix, old_json, new_json, details_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        pr_id,
                        snapshot_id,
                        event.kind,
                        event.changed_at_unix,
                        old_json,
                        new_json,
                        details_json
                    ],
                )
                .map_err(|source| self.query_error(source))?;
        }
        Ok(())
    }

    fn latest_pull_request_snapshot(
        &self,
        repository: &GitHubRepository,
        number: u64,
    ) -> Result<Option<StoredPullRequestSnapshot>, RepositoryError> {
        let row = self
            .connection
            .query_row(
                "SELECT pull_requests.id, snapshots.payload_json
                 FROM pull_request_snapshots snapshots
                 JOIN pull_requests pull_requests ON pull_requests.id = snapshots.pr_id
                 JOIN repositories repositories ON repositories.id = pull_requests.repository_id
                 WHERE repositories.host = 'github.com'
                   AND repositories.owner = ?1
                   AND repositories.name = ?2
                   AND pull_requests.number = ?3
                 ORDER BY snapshots.id DESC
                 LIMIT 1",
                params![&repository.owner, &repository.name, number as i64],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|source| self.query_error(source))?;
        row.map(|(pr_id, payload)| self.decode_snapshot(pr_id, &payload))
            .transpose()
    }

    fn latest_pull_request_snapshot_for_pr(
        &self,
        pr_id: i64,
    ) -> Result<Option<StoredPullRequestSnapshot>, RepositoryError> {
        let row = self
            .connection
            .query_row(
                "SELECT payload_json
                 FROM pull_request_snapshots
                 WHERE pr_id = ?1
                 ORDER BY id DESC
                 LIMIT 1",
                params![pr_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|source| self.query_error(source))?;
        row.map(|payload| self.decode_snapshot(pr_id, &payload))
            .transpose()
    }

    fn pull_request_history(
        &self,
        pr_id: i64,
    ) -> Result<Vec<PullRequestHistoryRecord>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT kind, changed_at_unix, old_json, new_json, details_json
                 FROM pull_request_history
                 WHERE pr_id = ?1
                 ORDER BY changed_at_unix, id",
            )
            .map_err(|source| self.query_error(source))?;
        let rows = statement
            .query_map(params![pr_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|source| self.query_error(source))?;
        let mut history = Vec::new();
        for row in rows {
            let (kind, changed_at_unix, old_json, new_json, details_json) =
                row.map_err(|source| self.query_error(source))?;
            history.push(PullRequestHistoryRecord {
                kind,
                changed_at_unix,
                old_json: self.decode_optional_json(old_json)?,
                new_json: self.decode_optional_json(new_json)?,
                details_json: self.decode_json(&details_json)?,
            });
        }
        Ok(history)
    }

    fn pull_request_actions(
        &self,
        pr_id: i64,
    ) -> Result<Vec<PullRequestActionRecord>, RepositoryError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT action, source, reason, changed_at_unix, details_json
                 FROM pull_request_actions
                 WHERE pr_id = ?1
                 ORDER BY changed_at_unix, id",
            )
            .map_err(|source| self.query_error(source))?;
        let rows = statement
            .query_map(params![pr_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|source| self.query_error(source))?;
        let mut actions = Vec::new();
        for row in rows {
            let (action, source, reason, changed_at_unix, details_json) =
                row.map_err(|source| self.query_error(source))?;
            actions.push(PullRequestActionRecord {
                action,
                source,
                reason,
                changed_at_unix,
                details_json: self.decode_json(&details_json)?,
            });
        }
        Ok(actions)
    }

    fn decode_snapshot(
        &self,
        pr_id: i64,
        payload: &str,
    ) -> Result<StoredPullRequestSnapshot, RepositoryError> {
        let status = serde_json::from_str(payload).map_err(|source| {
            RepositoryError::PullRequestSnapshotDecode {
                file: self.path.clone(),
                source,
            }
        })?;
        Ok(StoredPullRequestSnapshot { pr_id, status })
    }

    fn decode_optional_json(
        &self,
        value: Option<String>,
    ) -> Result<Option<serde_json::Value>, RepositoryError> {
        value.map(|value| self.decode_json(&value)).transpose()
    }

    fn decode_json(&self, value: &str) -> Result<serde_json::Value, RepositoryError> {
        serde_json::from_str(value).map_err(|source| RepositoryError::PullRequestStoreJsonDecode {
            file: self.path.clone(),
            source,
        })
    }

    fn query_error(&self, source: rusqlite::Error) -> RepositoryError {
        RepositoryError::PullRequestStoreQuery {
            file: self.path.clone(),
            source,
        }
    }

    /// Latest applied schema migration version for this store.
    pub fn schema_version(&self) -> Result<i64, RepositoryError> {
        self.connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(|source| RepositoryError::PullRequestStoreMigration {
                file: self.path.clone(),
                source,
            })
    }

    #[cfg(test)]
    pub(crate) fn connection(&self) -> &rusqlite::Connection {
        &self.connection
    }
}

fn pull_request_history_events(
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) -> Vec<PullRequestHistoryEvent> {
    let mut events = Vec::new();
    if previous.is_none() {
        events.push(PullRequestHistoryEvent {
            kind: "first_seen",
            changed_at_unix: observed_at_unix,
            old_json: None,
            new_json: Some(serde_json::json!({
                "number": current.number,
                "headOid": current.latest_commit_oid.as_deref(),
                "draft": current.draft,
                "closed": current.closed,
                "merged": current.merged,
            })),
            details_json: serde_json::json!({}),
        });
    }

    record_head_change(&mut events, previous, current, observed_at_unix);
    record_draft_change(&mut events, previous, current, observed_at_unix);
    record_lifecycle_change(&mut events, previous, current, observed_at_unix);
    record_reviewer_request_changes(&mut events, previous, current, observed_at_unix);
    record_review_state_changes(&mut events, previous, current, observed_at_unix);
    record_author_responses(&mut events, previous, current, observed_at_unix);
    record_reviewer_mentions(&mut events, previous, current, observed_at_unix);
    events
}

fn record_head_change(
    events: &mut Vec<PullRequestHistoryEvent>,
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) {
    let Some(previous) = previous else {
        return;
    };
    if previous.latest_commit_oid == current.latest_commit_oid {
        return;
    }
    events.push(PullRequestHistoryEvent {
        kind: "head_changed",
        changed_at_unix: observed_at_unix,
        old_json: Some(serde_json::json!({ "headOid": previous.latest_commit_oid.as_deref() })),
        new_json: Some(serde_json::json!({ "headOid": current.latest_commit_oid.as_deref() })),
        details_json: serde_json::json!({}),
    });
}

fn record_draft_change(
    events: &mut Vec<PullRequestHistoryEvent>,
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) {
    let Some(previous) = previous else {
        return;
    };
    if previous.draft == current.draft {
        return;
    }
    events.push(PullRequestHistoryEvent {
        kind: "draft_changed",
        changed_at_unix: draft_change_timestamp(current).unwrap_or(observed_at_unix),
        old_json: Some(serde_json::json!({ "draft": previous.draft })),
        new_json: Some(serde_json::json!({ "draft": current.draft })),
        details_json: serde_json::json!({}),
    });
}

fn record_lifecycle_change(
    events: &mut Vec<PullRequestHistoryEvent>,
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) {
    let Some(previous) = previous else {
        return;
    };
    if !previous.closed && current.closed {
        let kind = if current.merged { "merged" } else { "closed" };
        let changed_at_unix = if current.merged {
            current
                .merged_at
                .as_deref()
                .and_then(parse_github_timestamp_unix)
        } else {
            current
                .closed_at
                .as_deref()
                .and_then(parse_github_timestamp_unix)
        }
        .unwrap_or(observed_at_unix);
        events.push(PullRequestHistoryEvent {
            kind,
            changed_at_unix,
            old_json: Some(
                serde_json::json!({ "closed": previous.closed, "merged": previous.merged }),
            ),
            new_json: Some(
                serde_json::json!({ "closed": current.closed, "merged": current.merged }),
            ),
            details_json: serde_json::json!({}),
        });
    } else if previous.closed && !current.closed {
        events.push(PullRequestHistoryEvent {
            kind: "reopened",
            changed_at_unix: observed_at_unix,
            old_json: Some(
                serde_json::json!({ "closed": previous.closed, "merged": previous.merged }),
            ),
            new_json: Some(
                serde_json::json!({ "closed": current.closed, "merged": current.merged }),
            ),
            details_json: serde_json::json!({}),
        });
    }
}

fn record_reviewer_request_changes(
    events: &mut Vec<PullRequestHistoryEvent>,
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) {
    let previous_reviewers = previous.map(requested_reviewers).unwrap_or_default();
    let current_reviewers = requested_reviewers(current);
    for reviewer in current_reviewers.difference(&previous_reviewers) {
        events.push(PullRequestHistoryEvent {
            kind: "reviewer_requested",
            changed_at_unix: review_request_timestamp(current, reviewer)
                .unwrap_or(observed_at_unix),
            old_json: None,
            new_json: Some(reviewer_json(reviewer)),
            details_json: serde_json::json!({}),
        });
    }
    for reviewer in previous_reviewers.difference(&current_reviewers) {
        events.push(PullRequestHistoryEvent {
            kind: "review_request_removed",
            changed_at_unix: observed_at_unix,
            old_json: Some(reviewer_json(reviewer)),
            new_json: None,
            details_json: serde_json::json!({}),
        });
    }
}

fn record_review_state_changes(
    events: &mut Vec<PullRequestHistoryEvent>,
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) {
    let previous_states = previous.map(review_states).unwrap_or_default();
    let current_states = review_states(current);
    let reviewers = previous_states
        .keys()
        .chain(current_states.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for reviewer in reviewers {
        let previous_state = previous_states.get(&reviewer);
        let current_state = current_states.get(&reviewer);
        if previous_state == current_state {
            continue;
        }
        events.push(PullRequestHistoryEvent {
            kind: "review_state_changed",
            changed_at_unix: reviewer_activity_timestamp(current, &reviewer)
                .unwrap_or(observed_at_unix),
            old_json: previous_state
                .map(|state| serde_json::json!({ "reviewer": reviewer.as_str(), "state": state })),
            new_json: current_state
                .map(|state| serde_json::json!({ "reviewer": reviewer.as_str(), "state": state })),
            details_json: serde_json::json!({}),
        });
    }
}

fn record_author_responses(
    events: &mut Vec<PullRequestHistoryEvent>,
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) {
    let previous_responses = previous.map(author_response_keys).unwrap_or_default();
    for response in &current.reviewer_responses {
        let key = (
            response.reviewer.clone(),
            response.responded_at.clone(),
            response.body_text.clone(),
        );
        if previous_responses.contains(&key) {
            continue;
        }
        events.push(PullRequestHistoryEvent {
            kind: "author_response",
            changed_at_unix: parse_github_timestamp_unix(&response.responded_at)
                .unwrap_or(observed_at_unix),
            old_json: None,
            new_json: Some(serde_json::json!({
                "reviewer": &response.reviewer,
                "respondedAt": &response.responded_at,
                "bodyText": &response.body_text,
            })),
            details_json: serde_json::json!({}),
        });
    }
}

fn record_reviewer_mentions(
    events: &mut Vec<PullRequestHistoryEvent>,
    previous: Option<&PullRequestStatusRecord>,
    current: &PullRequestStatusRecord,
    observed_at_unix: i64,
) {
    let previous_mentions = previous.map(reviewer_mention_keys).unwrap_or_default();
    for mention in &current.reviewer_mentions {
        let key = (mention.reviewer.clone(), mention.mentioned_at.clone());
        if previous_mentions.contains(&key) {
            continue;
        }
        events.push(PullRequestHistoryEvent {
            kind: "reviewer_mentioned",
            changed_at_unix: parse_github_timestamp_unix(&mention.mentioned_at)
                .unwrap_or(observed_at_unix),
            old_json: None,
            new_json: Some(serde_json::json!({
                "reviewer": &mention.reviewer,
                "mentionedAt": &mention.mentioned_at,
            })),
            details_json: serde_json::json!({}),
        });
    }
}

fn requested_reviewers(status: &PullRequestStatusRecord) -> BTreeSet<String> {
    status
        .requested_reviewers
        .users
        .iter()
        .map(|reviewer| format!("user:{reviewer}"))
        .chain(
            status
                .requested_reviewers
                .teams
                .iter()
                .map(|reviewer| format!("team:{reviewer}")),
        )
        .collect()
}

fn reviewer_json(reviewer: &str) -> serde_json::Value {
    if let Some(login) = reviewer.strip_prefix("user:") {
        serde_json::json!({ "type": "user", "login": login })
    } else if let Some(slug) = reviewer.strip_prefix("team:") {
        serde_json::json!({ "type": "team", "slug": slug })
    } else {
        serde_json::json!({ "type": "unknown", "value": reviewer })
    }
}

fn review_states(status: &PullRequestStatusRecord) -> BTreeMap<String, &'static str> {
    let mut states = BTreeMap::new();
    for reviewer in &status.addressed_reviewers {
        states.insert(reviewer.clone(), "addressed");
    }
    for reviewer in &status.commented_reviewers {
        states.insert(reviewer.clone(), "commented");
    }
    for reviewer in &status.approved_reviewers {
        states.insert(reviewer.clone(), "approved");
    }
    for reviewer in &status.changes_requested_reviewers {
        states.insert(reviewer.clone(), "changes_requested");
    }
    for reviewer in &status.dismissed_reviewers {
        states.insert(reviewer.clone(), "dismissed");
    }
    states
}

fn author_response_keys(status: &PullRequestStatusRecord) -> BTreeSet<(String, String, String)> {
    status
        .reviewer_responses
        .iter()
        .map(|response| {
            (
                response.reviewer.clone(),
                response.responded_at.clone(),
                response.body_text.clone(),
            )
        })
        .collect()
}

fn reviewer_mention_keys(status: &PullRequestStatusRecord) -> BTreeSet<(String, String)> {
    status
        .reviewer_mentions
        .iter()
        .map(|mention| (mention.reviewer.clone(), mention.mentioned_at.clone()))
        .collect()
}

fn draft_change_timestamp(status: &PullRequestStatusRecord) -> Option<i64> {
    let target = if status.draft {
        PullRequestTimelineEventKind::ConvertToDraft
    } else {
        PullRequestTimelineEventKind::ReadyForReview
    };
    status
        .timeline_events
        .iter()
        .filter(|event| event.kind == target)
        .filter_map(|event| parse_github_timestamp_unix(&event.created_at))
        .max()
}

fn review_request_timestamp(status: &PullRequestStatusRecord, reviewer: &str) -> Option<i64> {
    let reviewer = if let Some(login) = reviewer.strip_prefix("user:") {
        login.to_owned()
    } else if let Some(team) = reviewer.strip_prefix("team:") {
        format!("team/{team}")
    } else {
        return None;
    };
    status
        .timeline_events
        .iter()
        .filter(|event| event.kind == PullRequestTimelineEventKind::ReviewRequested)
        .filter(|event| event.reviewer.as_deref() == Some(reviewer.as_str()))
        .filter_map(|event| parse_github_timestamp_unix(&event.created_at))
        .max()
}

fn reviewer_activity_timestamp(status: &PullRequestStatusRecord, reviewer: &str) -> Option<i64> {
    status
        .review_activity
        .iter()
        .filter(|activity| activity.reviewer == reviewer)
        .filter_map(|activity| parse_github_timestamp_unix(&activity.reviewed_at))
        .max()
}

fn parse_github_timestamp_unix(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.timestamp())
}

fn unique_numbers(numbers: &[u64]) -> Vec<u64> {
    let mut unique = Vec::new();
    let mut seen = BTreeSet::new();
    for number in numbers {
        if seen.insert(*number) {
            unique.push(*number);
        }
    }
    unique
}

fn stable_payload_hash(payload: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let hash = payload.as_bytes().iter().fold(FNV_OFFSET, |hash, byte| {
        let hash = hash ^ u64::from(*byte);
        hash.wrapping_mul(FNV_PRIME)
    });
    format!("{hash:016x}")
}

pub fn pull_request_store_file(
    environment: &RuntimeEnvironment,
) -> Result<PathBuf, RepositoryError> {
    let root = environment
        .variable("XDG_STATE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .home_dir()
                .map(|home| home.join(".local").join("state"))
        })
        .ok_or_else(|| RepositoryError::InvalidConfig {
            file: "environment".to_owned(),
            message: "HOME or XDG_STATE_HOME must be set to locate the pull-request store"
                .to_owned(),
        })?;
    Ok(root.join("jx").join(PULL_REQUEST_STORE_FILE))
}

fn migrate_pull_request_store(
    connection: &rusqlite::Connection,
    path: &Path,
) -> Result<(), RepositoryError> {
    connection
        .execute_batch(CREATE_PULL_REQUEST_STORE_SCHEMA)
        .map_err(|source| RepositoryError::PullRequestStoreMigration {
            file: path.to_path_buf(),
            source,
        })?;

    let applied_at_unix = chrono::Utc::now().timestamp();
    connection
        .execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at_unix) VALUES (?1, ?2)",
            rusqlite::params![PULL_REQUEST_STORE_SCHEMA_VERSION, applied_at_unix],
        )
        .map_err(|source| RepositoryError::PullRequestStoreMigration {
            file: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn pull_request_store_schema_sql() -> &'static str {
    CREATE_PULL_REQUEST_STORE_SCHEMA
}
