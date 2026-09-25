use super::*;

/// Canonicalizes review facts shared by summary and full queries, ignoring connection ordering.
/// Capture this from the full response too, so a review arriving between queries is not lost.
pub(super) fn review_refresh_key(
    decision: Option<&str>,
    requests: &GraphQlReviewRequests,
    reviews: &GraphQlReviews,
) -> String {
    let mut requested = requests
        .nodes
        .iter()
        .map(|node| {
            node.requested_reviewer
                .as_ref()
                .map(|reviewer| (&reviewer.type_name, &reviewer.login, &reviewer.slug))
        })
        .collect::<Vec<_>>();
    requested.sort_unstable();
    let mut latest_reviews = reviews
        .nodes
        .iter()
        .map(|review| {
            (
                review
                    .author
                    .as_ref()
                    .map(|author| (&author.type_name, &author.login)),
                &review.state,
                &review.submitted_at,
                &review.author_association,
            )
        })
        .collect::<Vec<_>>();
    latest_reviews.sort_unstable();
    serde_json::to_string(&(decision, requests.total_count, requested, latest_reviews))
        .expect("review freshness facts serialize")
}

// Both queries must capture the same facts; avoid loading discussion bodies in the summary.
pub(super) const REVIEW_REFRESH_FRAGMENT: &str = r#"fragment PullRequestReviewRefreshFields on PullRequest {
  reviewDecision
  reviewRequests(first: 100) {
    totalCount
    nodes {
      requestedReviewer {
        __typename
        ... on User {
          login
        }
      }
    }
  }
  latestReviews(first: 100) {
    nodes {
      state
      submittedAt
      author {
        __typename
        login
      }
      authorAssociation
    }
  }
}
"#;

#[cfg(test)]
#[path = "../tests/review_refresh.rs"]
mod tests;
