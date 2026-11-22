use serde::{Deserialize,
            Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageScope {
    Turn,
    Session,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Usage {
    pub scope: UsageScope,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub cost_usd: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UsageReport {
    pub usage: Option<Usage>,
    pub reported_turns: usize,
    pub missing_turns: usize,
}
impl UsageReport {
    pub fn recorded(timeline: &[crate::TimelineEntry], turns: usize) -> Self {
        let mut latest = std::collections::BTreeMap::new();
        let mut session = None;
        for entry in timeline {
            if let crate::Event::Usage {
                usage,
            } = &entry.event
            {
                latest.insert(entry.message_id, usage);
                if usage.scope == UsageScope::Session {
                    session = Some(usage.clone());
                }
            }
        }
        let reported_turns = latest.len();
        let usage = if let Some(session) = session {
            Some(session)
        } else if let Some(last) = latest.values().last() {
            let sum = |field: fn(&Usage) -> Option<u64>| latest.values().try_fold(0u64, |total, usage| total.checked_add(field(usage)?));
            Some(Usage {
                scope: UsageScope::Session,
                input_tokens: sum(|u| u.input_tokens),
                output_tokens: sum(|u| u.output_tokens),
                cached_input_tokens: sum(|u| u.cached_input_tokens),
                cache_write_tokens: sum(|u| u.cache_write_tokens),
                total_tokens: sum(|u| u.total_tokens),
                context_window: last.context_window,
                cost_usd: latest.values().try_fold(0.0, |total, u| {
                    let sum = total + u.cost_usd?;
                    sum.is_finite().then_some(sum)
                }),
            })
        } else {
            None
        };
        Self {
            usage,
            reported_turns,
            missing_turns: turns.saturating_sub(reported_turns),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deduplicates_snapshots_and_never_adds_cumulative_usage() {
        let usage = Usage {
            scope: UsageScope::Turn,
            input_tokens: Some(10),
            output_tokens: Some(2),
            total_tokens: Some(12),
            cached_input_tokens: None,
            cache_write_tokens: None,
            context_window: None,
            cost_usd: Some(0.01),
        };
        let mut events = vec![
            crate::TimelineEntry {
                message_id: 1,
                event: crate::Event::Usage {
                    usage: usage.clone(),
                },
            },
            crate::TimelineEntry {
                message_id: 1,
                event: crate::Event::Usage {
                    usage: usage.clone(),
                },
            },
        ];
        assert_eq!(UsageReport::recorded(&events, 2).usage.unwrap().input_tokens, Some(10));
        events.push(crate::TimelineEntry {
            message_id: 2,
            event: crate::Event::Usage {
                usage: usage.clone(),
            },
        });
        assert_eq!(UsageReport::recorded(&events, 2).usage.unwrap().total_tokens, Some(24));
        let mut usage = usage;
        usage.scope = UsageScope::Session;
        usage.total_tokens = Some(99);
        events.push(crate::TimelineEntry {
            message_id: 2,
            event: crate::Event::Usage {
                usage,
            },
        });
        assert_eq!(UsageReport::recorded(&events, 3).usage.unwrap().total_tokens, Some(99));
        assert_eq!(UsageReport::recorded(&events, 3).missing_turns, 1);
    }
}
