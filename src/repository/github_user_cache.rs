use super::*;
use chrono::{DateTime, Duration, Utc};

const CACHE_VERSION: u32 = 1;
const GITHUB_USER_CACHE_RELATIVE_PATH: [&str; 2] = ["jx", "github-users.toml"];
pub const GITHUB_USER_NAME_CACHE_TTL_DAYS: i64 = 180;

/// Long-lived cache of GitHub login-to-display-name lookups for human output.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct GitHubUserNameCache {
    #[serde(default = "default_cache_version")]
    pub version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub users: BTreeMap<String, GitHubUserNameCacheEntry>,
}

/// One cached GitHub public profile name and its refresh timestamp.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct GitHubUserNameCacheEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub updated_at: String,
}

impl Default for GitHubUserNameCache {
    fn default() -> Self {
        Self {
            version: CACHE_VERSION,
            users: BTreeMap::new(),
        }
    }
}

impl GitHubUserNameCache {
    /// Returns a fresh cached name result, including cached missing names.
    pub fn fresh_name(&self, login: &str, now: DateTime<Utc>) -> Option<Option<String>> {
        let entry = self.users.get(login)?;
        if !entry.is_fresh(now) {
            return None;
        }
        Some(entry.name.clone())
    }

    /// Returns any cached name result, even after the refresh window expires.
    pub fn cached_name(&self, login: &str) -> Option<Option<String>> {
        self.users.get(login).map(|entry| entry.name.clone())
    }

    /// Stores a freshly fetched public profile name for future human rendering.
    pub fn upsert(&mut self, login: &str, name: Option<&str>, now: DateTime<Utc>) {
        let login = login.trim();
        if login.is_empty() {
            return;
        }
        let name = name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        self.version = CACHE_VERSION;
        self.users.insert(
            login.to_owned(),
            GitHubUserNameCacheEntry {
                name,
                updated_at: now.to_rfc3339(),
            },
        );
    }
}

impl GitHubUserNameCacheEntry {
    fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        DateTime::parse_from_rfc3339(&self.updated_at)
            .map(|updated_at| {
                now.signed_duration_since(updated_at.with_timezone(&Utc))
                    <= Duration::days(GITHUB_USER_NAME_CACHE_TTL_DAYS)
            })
            .unwrap_or(false)
    }
}

/// Reads the global GitHub user-name cache, treating a missing cache as empty.
pub fn read_github_user_name_cache(
    environment: &RuntimeEnvironment,
) -> Result<GitHubUserNameCache, RepositoryError> {
    let Some(file) = github_user_name_cache_file(environment) else {
        return Ok(GitHubUserNameCache::default());
    };
    let contents = match fs::read_to_string(&file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(GitHubUserNameCache::default());
        }
        Err(source) => return Err(RepositoryError::CacheRead { file, source }),
    };
    toml::from_str(&contents).map_err(|source| RepositoryError::CacheParse { file, source })
}

/// Writes the global GitHub user-name cache under the operator cache directory.
pub fn write_github_user_name_cache(
    environment: &RuntimeEnvironment,
    cache: &GitHubUserNameCache,
) -> Result<(), RepositoryError> {
    let Some(file) = github_user_name_cache_file(environment) else {
        return Ok(());
    };
    let Some(directory) = file.parent() else {
        return Ok(());
    };
    fs::create_dir_all(directory).map_err(|source| RepositoryError::CacheWrite {
        file: directory.to_path_buf(),
        source,
    })?;
    let contents = toml::to_string(cache).expect("GitHub user-name cache serializes");
    let temporary = file.with_extension("toml.tmp");
    fs::write(&temporary, contents).map_err(|source| RepositoryError::CacheWrite {
        file: temporary.clone(),
        source,
    })?;
    fs::rename(&temporary, &file).map_err(|source| RepositoryError::CacheWrite { file, source })
}

fn github_user_name_cache_file(environment: &RuntimeEnvironment) -> Option<PathBuf> {
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
        GITHUB_USER_CACHE_RELATIVE_PATH
            .iter()
            .fold(cache_root, |path, component| path.join(component)),
    )
}

fn default_cache_version() -> u32 {
    CACHE_VERSION
}
