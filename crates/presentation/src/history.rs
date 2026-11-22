use gpui_agent_contracts::{ConversationSummary,
                           Job,
                           JobState,
                           Provider};

pub struct HistoryGroup {
    pub provider: Provider,
    pub account_id: String,
    pub conversations: Vec<ConversationSummary>,
}

pub struct HistoryEntry<'a> {
    pub provider: Provider,
    pub account_id: &'a str,
    pub conversation: Option<&'a ConversationSummary>,
    pub job: Option<&'a Job>,
}

pub fn history_entries<'a>(groups: &'a [HistoryGroup], jobs: &'a [Job], workspace: i64, archived: bool, query: &str) -> Vec<HistoryEntry<'a>> {
    let mut jobs: Vec<_> = jobs.iter().filter(|job| job.scope.workspace_id == Some(workspace)).collect();
    jobs.sort_by_key(|job| {
        (
            match job.state {
                | JobState::Waiting => 0,
                | JobState::Running => 1,
                | JobState::Queued => 2,
                | _ => 3,
            },
            std::cmp::Reverse(job.id),
        )
    });
    // First job per (provider, account, conversation) in display order.
    let mut by_conversation: std::collections::HashMap<(usize, &str, Option<i64>), &Job> = std::collections::HashMap::new();
    for job in &jobs {
        by_conversation
            .entry((
                gpui_agent_contracts::provider_index(job.scope.provider),
                job.scope.account_id.as_str(),
                job.conversation_id,
            ))
            .or_insert(job);
    }
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for group in groups {
        for conversation in &group.conversations {
            let key = (
                gpui_agent_contracts::provider_index(group.provider),
                group.account_id.as_str(),
                Some(conversation.id),
            );
            seen.insert(key);
            if conversation.archived != archived {
                continue;
            }
            entries.push(HistoryEntry {
                provider: group.provider,
                account_id: &group.account_id,
                conversation: Some(conversation),
                job: by_conversation.get(&key).copied(),
            });
        }
    }
    if !archived {
        for job in jobs {
            let key = (
                gpui_agent_contracts::provider_index(job.scope.provider),
                job.scope.account_id.as_str(),
                job.conversation_id,
            );
            if job.conversation_id.is_some() && !seen.insert(key) {
                continue;
            }
            if !query.is_empty() && gpui_agent_contracts::match_ranges(&job.preview, query).is_empty() {
                continue;
            }
            entries.push(HistoryEntry {
                provider: job.scope.provider,
                account_id: &job.scope.account_id,
                conversation: None,
                job: Some(job),
            });
        }
    }
    entries.sort_by_key(|entry| {
        (
            std::cmp::Reverse(entry.conversation.is_some_and(|c| c.pinned)),
            std::cmp::Reverse(entry.conversation.map_or(0, |c| c.updated_at).max(entry.job.map_or(0, |job| job.created_at))),
            std::cmp::Reverse(entry.conversation.map_or_else(|| entry.job.map_or(0, |job| job.id), |c| c.id)),
        )
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_agent_contracts::{JobScope,
                               JobState};

    fn job(id: i64, workspace: i64, account: &str, conversation_id: Option<i64>, state: JobState) -> Job {
        Job {
            id,
            scope: JobScope {
                provider: Provider::Claude,
                account_id: account.into(),
                workspace_id: Some(workspace),
                workspace_path: String::new(),
            },
            state,
            conversation_id,
            operation_id: None,
            pending_request_id: None,
            last_sequence: 0,
            preview: format!("Job {id}"),
            cancel_requested: false,
            error: None,
            created_at: id,
        }
    }

    #[test]
    fn existing_project_merges_jobs_and_conversations_without_cross_scope_duplicates() {
        // Given saved chats and multiple jobs for one chat, including another
        // project and account.
        let groups = vec![HistoryGroup {
            provider: Provider::Claude,
            account_id: "default".into(),
            conversations: serde_json::from_str(r#"[{"id":7,"title":"Saved chat","updated_at":10},{"id":9,"title":"Chat only","updated_at":5}]"#).unwrap(),
        }];
        let jobs = vec![
            job(1, 42, "default", Some(7), JobState::Completed),
            job(2, 42, "default", Some(7), JobState::Waiting),
            job(3, 43, "default", Some(7), JobState::Running),
            job(4, 42, "work", Some(7), JobState::Completed),
            job(5, 42, "default", None, JobState::Queued),
        ];
        // When the project's sidebar is built, then each chat appears once with
        // its live job state.
        let entries = history_entries(&groups, &jobs, 42, false, "");
        assert_eq!(entries.len(), 4);
        let saved = entries.iter().find(|entry| entry.conversation.is_some_and(|c| c.id == 7)).unwrap();
        assert_eq!(saved.job.unwrap().state, JobState::Waiting);
        assert!(entries.iter().any(|entry| entry.account_id == "work" && entry.conversation.is_none()));
        assert!(entries.iter().any(|entry| entry.job.is_some_and(|job| job.id == 5)));
        assert!(!entries.iter().any(|entry| entry.job.is_some_and(|job| job.scope.workspace_id == Some(43))));
    }
}
