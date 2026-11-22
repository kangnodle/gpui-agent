use gpui_agent_domain::{Event,
                        Usage,
                        UsageScope};
use serde_json::Value;
pub(crate) fn claude(message: &Value) -> Option<Event> {
    let raw = &message["usage"];
    let input = raw["input_tokens"].as_u64();
    let output = raw["output_tokens"].as_u64();
    let cached = raw["cache_read_input_tokens"].as_u64();
    let write = raw["cache_creation_input_tokens"].as_u64();
    let cost = message["total_cost_usd"].as_f64().filter(|n| n.is_finite() && *n >= 0.0);
    if [input, output, cached, write].iter().all(Option::is_none) && cost.is_none() {
        return None;
    }
    Some(Event::Usage {
        usage: Usage {
            scope: UsageScope::Turn,
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: cached,
            cache_write_tokens: write,
            total_tokens: input
                .zip(output)
                .zip(cached)
                .zip(write)
                .and_then(|(((i, o), c), w)| i.checked_add(o)?.checked_add(c)?.checked_add(w)),
            cost_usd: cost,
            context_window: None,
        },
    })
}
pub(crate) fn codex(message: &Value) -> Option<Event> {
    if message["method"] != "thread/tokenUsage/updated" {
        return None;
    }
    let raw = &message["params"]["tokenUsage"];
    let total = &raw["total"];
    if !["inputTokens", "outputTokens", "cachedInputTokens", "totalTokens"]
        .iter()
        .any(|key| total[key].as_u64().is_some())
    {
        return None;
    }
    Some(Event::Usage {
        usage: Usage {
            scope: UsageScope::Session,
            input_tokens: total["inputTokens"].as_u64(),
            output_tokens: total["outputTokens"].as_u64(),
            cached_input_tokens: total["cachedInputTokens"].as_u64(),
            cache_write_tokens: None,
            total_tokens: total["totalTokens"].as_u64(),
            context_window: raw["modelContextWindow"].as_u64(),
            cost_usd: None,
        },
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn parses_only_reported_provider_fields() {
        let Some(Event::Usage {
            usage,
        }) = claude(&json!({"usage":{"input_tokens":10,"output_tokens":2,"cache_read_input_tokens":30,"cache_creation_input_tokens":4},"total_cost_usd":0.2}))
        else {
            panic!()
        };
        assert_eq!(usage.total_tokens, Some(46));
        assert_eq!(usage.scope, UsageScope::Turn);
        let Some(Event::Usage {
            usage,
        }) = codex(
            &json!({"method":"thread/tokenUsage/updated","params":{"tokenUsage":{"total":{"inputTokens":40,"outputTokens":2,"cachedInputTokens":30,"totalTokens":42},"last":{"totalTokens":1},"modelContextWindow":100}}}),
        )
        else {
            panic!()
        };
        assert_eq!(usage.total_tokens, Some(42));
        assert_eq!(usage.cost_usd, None);
        assert_eq!(usage.scope, UsageScope::Session);
        let Some(Event::Usage {
            usage,
        }) = claude(&json!({"usage":{"input_tokens":1,"output_tokens":2}}))
        else {
            panic!()
        };
        assert!(usage.total_tokens.is_none());
        let Some(Event::Usage {
            usage,
        }) = claude(&json!({"total_cost_usd":0.02}))
        else {
            panic!()
        };
        assert_eq!(usage.cost_usd, Some(0.02));
        assert!(usage.input_tokens.is_none());
        assert!(claude(&json!({})).is_none());
        assert!(codex(&json!({"method":"other"})).is_none());
    }
}
