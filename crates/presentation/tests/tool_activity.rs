use gpui_agent_contracts::{Event,
                           TimelineEntry};
use gpui_agent_presentation::tools;
use serde_json::json;

#[test]
fn given_replayed_host_check_updates_when_projected_then_each_job_attempt_shows_its_latest_receipt_once() {
    let entries:Vec<gpui_agent_contracts::TimelineEntry>=serde_json::from_value(json!([
        {"message_id":2,"event":{"type":"verification","report":{"job_id":7,"attempt":1,"revision":{"head":"abc","fingerprint":"tree"},"finished_revision":null,"checks":[{"command":["bun","test"],"status":"running","exit_code":null,"output":""}]}}},
        {"message_id":2,"event":{"type":"verification","report":{"job_id":7,"attempt":1,"revision":{"head":"abc","fingerprint":"tree"},"finished_revision":{"head":"abc","fingerprint":"tree"},"checks":[{"command":["bun","test"],"status":"passed","exit_code":0,"output":"2 passed"}]}}}
    ])).unwrap();
    let reports = gpui_agent_presentation::verification_reports(&entries, 2);
    assert_eq!(reports.len(), 1);
    assert!(reports[0].passed());
    assert!(gpui_agent_presentation::verification_reports(&entries, 3).is_empty());
}

#[test]
fn given_mixed_timeline_events_when_routed_then_execution_is_separate_from_user_attention() {
    let events: Vec<Event> = serde_json::from_value(json!([
        {"type":"tool","call":{"id":"read","status":"in_progress"}},
        {"type":"input_delivery","message_id":4,"delivered":true},
        {"type":"runtime_status","status":"retrying","message":"Reconnecting","code":null},
        {"type":"permission","method":"fixture","details":{}},
        {"type":"user_input","questions":[]},
        {"type":"elicitation","server_name":"fixture","message":"Confirm","schema":null,"url":null},
        {"type":"interaction_resolved","request_id":9,"response":{"type":"approval","allow":true}},
        {"type":"runtime_status","status":"warning","message":"Review needed","code":null},
        {"type":"runtime_status","status":"error","message":"Disconnected","code":null},
        {"type":"runtime_status","status":"usage_limited","message":"Limit reached","code":null}
    ]))
    .unwrap();
    let work: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, event)| gpui_agent_presentation::is_work_event(event))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(work, vec![0, 1, 2]);
}

#[test]
fn given_tool_updates_when_a_turn_ends_then_activity_is_retained_once_and_patch_snapshots_replace_input() {
    let events: Vec<Event> = serde_json::from_value(json!([
        {"type":"tool","call":{"id":"mcp","activity":[{"type":"progress","message":"Searching"}]}},
        {"type":"tool","call":{"id":"mcp","title":"lookup","status":"in_progress","activity":[{"type":"progress","message":"Checking"}]}},
        {"type":"tool","call":{"id":"mcp","status":"completed","output":{"text":"Found"}}},
        {"type":"tool","call":{"id":"terminal","output_delta":"partial stdout","activity":[{"type":"terminal_input","process_id":"42","stdin":"continue\n"}]}},
        {"type":"tool","call":{"id":"terminal","status":"completed","output":"actual stdout","activity":[{"type":"terminal_input","process_id":"42","stdin":""}]}},
        {"type":"tool","call":{"id":"patch","status":"in_progress","input":[{"path":"old.rs"}]}},
        {"type":"tool","call":{"id":"patch","input":[{"path":"new.rs","diff":"+new"}]}},
        {"type":"tool","call":{"id":"patch","input":[]}},
        {"type":"turn_end","completed":false}
    ])).unwrap();
    let entries: Vec<_> = events
        .into_iter()
        .map(|event| TimelineEntry {
            message_id: 7,
            event,
        })
        .collect();
    let calls = tools(&entries, 7);
    assert_eq!(calls.len(), 3);
    assert_eq!(
        serde_json::to_value(&calls[0].activity).unwrap(),
        json!([
            {"type":"progress","message":"Searching"},{"type":"progress","message":"Checking"}
        ])
    );
    assert_eq!(calls[0].output, Some(json!({"text":"Found"})));
    assert_eq!(calls[0].status.as_deref(), Some("completed"));
    assert_eq!(
        serde_json::to_value(&calls[1].activity).unwrap(),
        json!([
            {"type":"terminal_input","process_id":"42","stdin":"continue\n"}, {"type":"terminal_input","process_id":"42","stdin":""}
        ])
    );
    assert_eq!(calls[1].output, Some(json!("actual stdout")));
    assert_eq!(calls[2].input, Some(json!([])));
    assert_eq!(calls[2].status.as_deref(), Some("interrupted"));
    assert!(tools(&entries, 8).is_empty());
}
