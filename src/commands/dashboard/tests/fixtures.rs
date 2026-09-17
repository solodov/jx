use crate::domain::PrActionContext;
use crate::repository::GitHubRepository;

pub(in crate::commands) fn context(number: u64, repository: &str) -> PrActionContext {
    PrActionContext {
        repository: GitHubRepository::parse(&format!("https://github.com/{repository}")).unwrap(),
        repository_root: None,
        pr_number: number,
        pr_url: format!("https://github.com/{repository}/pull/{number}"),
        title: format!("Full title {number}"),
        branch: format!("feature-{number}"),
        base_branch: "main".to_owned(),
        head_oid: None,
        local_commit_id: None,
        local_change_id: None,
    }
}
