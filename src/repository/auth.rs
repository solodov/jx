use super::*;

const TOKEN_ENV_CANDIDATES: [&str; 3] = ["JX_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];

/// Supported GitHub token source. The token value is never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSource {
    Environment(&'static str),
    Keychain(KeychainConfig),
    Missing,
}

impl TokenSource {
    pub fn discover(environment: &RuntimeEnvironment, config: &WorkflowConfig) -> Self {
        TOKEN_ENV_CANDIDATES
            .iter()
            .copied()
            .find(|name| {
                environment
                    .variable(name)
                    .is_some_and(|value| !value.is_empty())
            })
            .map(Self::Environment)
            .or_else(|| config.auth.keychain.clone().map(Self::Keychain))
            .unwrap_or(Self::Missing)
    }

    /// Returns the token value for the discovered source without storing it in repository context.
    pub(crate) fn token(
        &self,
        environment: &RuntimeEnvironment,
    ) -> Result<Option<String>, TokenReadError> {
        match self {
            Self::Environment(name) => Ok(environment
                .variable(name)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)),
            Self::Keychain(config) => read_keychain_token(config).map(Some),
            Self::Missing => Ok(None),
        }
    }

    /// Human-readable token source status that does not reveal token values.
    pub fn summary(&self) -> String {
        match self {
            Self::Environment(name) => format!("{name} environment variable"),
            Self::Keychain(config) => format!(
                "keychain account `{account}` for service `{service}`",
                account = config.account,
                service = config.service
            ),
            Self::Missing => "not found".to_owned(),
        }
    }

    /// Stable non-secret key for caches whose entries are scoped to a token source.
    pub(crate) fn cache_key(&self) -> String {
        match self {
            Self::Environment(name) => format!("env:{name}"),
            Self::Keychain(config) => format!(
                "keychain:{service}:{account}",
                service = config.service,
                account = config.account
            ),
            Self::Missing => "missing".to_owned(),
        }
    }
}

/// Keychain entry used to load a GitHub token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeychainConfig {
    pub service: String,
    pub account: String,
}

/// Failures while reading a configured keychain token.
#[derive(Debug, Error)]
pub enum TokenReadError {
    #[error("No token found in keychain account `{account}` for service `{service}`. Add it to the OS keychain or set JX_GITHUB_TOKEN, GH_TOKEN, or GITHUB_TOKEN.")]
    KeychainMissing { service: String, account: String },
    #[error("Could not initialize keychain account `{account}` for service `{service}`: {source}")]
    KeychainEntry {
        service: String,
        account: String,
        source: keyring::Error,
    },
    #[error(
        "Could not read token from keychain account `{account}` for service `{service}`: {source}"
    )]
    KeychainRead {
        service: String,
        account: String,
        source: keyring::Error,
    },
}

fn read_keychain_token(config: &KeychainConfig) -> Result<String, TokenReadError> {
    let entry = keyring::Entry::new(&config.service, &config.account).map_err(|source| {
        TokenReadError::KeychainEntry {
            service: config.service.clone(),
            account: config.account.clone(),
            source,
        }
    })?;

    match entry.get_password() {
        Ok(token) if !token.is_empty() => Ok(token),
        Ok(_) | Err(keyring::Error::NoEntry) => Err(TokenReadError::KeychainMissing {
            service: config.service.clone(),
            account: config.account.clone(),
        }),
        Err(source) => Err(TokenReadError::KeychainRead {
            service: config.service.clone(),
            account: config.account.clone(),
            source,
        }),
    }
}
