use super::*;
use chrono::{DateTime, Duration, Utc};

const CACHE_VERSION: u32 = 1;
const GITHUB_AUTH_CACHE_RELATIVE_PATH: [&str; 2] = ["jx", "github-auth.toml"];
pub const GITHUB_AUTH_CACHE_TTL_DAYS: i64 = 30;

/// Long-lived cache of stable authenticated GitHub facts used to avoid publish preflights.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct GitHubAuthCache {
    #[serde(default = "default_cache_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tokens: BTreeMap<String, GitHubAuthCacheEntry>,
}

/// Cached authenticated user identity for one discovered token source.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct GitHubAuthCacheEntry {
    pub login: String,
    pub updated_at: String,
}

impl Default for GitHubAuthCache {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            tokens: BTreeMap::new(),
        }
    }
}

impl GitHubAuthCache {
    /// Returns the cached login while the stable auth fact is still fresh.
    pub fn fresh_login(&self, token_source: &TokenSource, now: DateTime<Utc>) -> Option<String> {
        let entry = self.tokens.get(&token_source.cache_key())?;
        if !entry.is_fresh(now) {
            return None;
        }
        Some(entry.login.clone())
    }

    /// Stores a freshly fetched authenticated login for this token source.
    pub fn upsert_login(
        &mut self,
        token_source: &TokenSource,
        login: impl Into<String>,
        now: DateTime<Utc>,
    ) {
        let login = login.into();
        if login.trim().is_empty() {
            return;
        }
        self.version = CACHE_VERSION;
        self.tokens.insert(
            token_source.cache_key(),
            GitHubAuthCacheEntry {
                login,
                updated_at: now.to_rfc3339(),
            },
        );
    }
}

impl GitHubAuthCacheEntry {
    fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        DateTime::parse_from_rfc3339(&self.updated_at)
            .map(|updated_at| {
                !self.login.trim().is_empty()
                    && now.signed_duration_since(updated_at.with_timezone(&Utc))
                        <= Duration::days(GITHUB_AUTH_CACHE_TTL_DAYS)
            })
            .unwrap_or(false)
    }
}

/// Reads the global GitHub auth cache, treating a missing cache as empty.
pub fn read_github_auth_cache(
    environment: &RuntimeEnvironment,
) -> Result<GitHubAuthCache, RepositoryError> {
    let Some(file) = github_auth_cache_file(environment) else {
        return Ok(GitHubAuthCache::default());
    };
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(GitHubAuthCache::default());
        }
        Err(source) => return Err(RepositoryError::CacheRead { file, source }),
    };
    toml::from_str(&contents).map_err(|source| RepositoryError::CacheParse { file, source })
}

/// Writes the global GitHub auth cache under the operator cache directory.
pub fn write_github_auth_cache(
    environment: &RuntimeEnvironment,
    cache: &GitHubAuthCache,
) -> Result<(), RepositoryError> {
    let Some(file) = github_auth_cache_file(environment) else {
        return Ok(());
    };
    let Some(directory) = file.parent() else {
        return Ok(());
    };
    fs::create_dir_all(directory).map_err(|source| RepositoryError::CacheWrite {
        file: directory.to_path_buf(),
        source,
    })?;
    let contents = toml::to_string(cache).expect("GitHub auth cache serializes");
    let temporary = file.with_extension("toml.tmp");
    fs::write(&temporary, contents).map_err(|source| RepositoryError::CacheWrite {
        file: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, &file).map_err(|source| RepositoryError::CacheWrite { file, source })
}

fn github_auth_cache_file(environment: &RuntimeEnvironment) -> Option<PathBuf> {
    let cache_root = environment
        .variable("XDG_CACHE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            environment
                .variable("HOME")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".cache"))
        })?;
    Some(
        GITHUB_AUTH_CACHE_RELATIVE_PATH
            .iter()
            .fold(cache_root, |path, component| path.join(component)),
    )
}

fn default_cache_version() -> u32 {
    CACHE_VERSION
}
