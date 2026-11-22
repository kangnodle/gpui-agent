//! Validation of responses to individual runtime requests.
pub fn validate_response(event: &crate::Event, response: &crate::InteractionResponse) -> Result<(), &'static str> {
    use crate::{Event,
                InteractionResponse};
    if matches!(
        response,
        InteractionResponse::Approval {
            allow: true
        }
    ) && !event.allows_once()
    {
        return Err("This request does not offer a one-time approval");
    }
    match (event, response) {
        | (
            Event::Elicitation {
                schema,
                url,
                ..
            },
            InteractionResponse::Elicitation {
                action,
                content,
            },
        ) => {
            if *action != crate::ElicitationAction::Accept {
                return if content.is_none() {
                    Ok(())
                } else {
                    Err("Declined requests cannot contain input")
                };
            }
            match (schema, url, content) {
                | (Some(schema), None, Some(content)) => crate::elicitation::validate_form(schema, content),
                | (None, Some(_), None) => Ok(()),
                | _ => Err("Response does not match the input request"),
            }
        },
        | (
            Event::Permission {
                method, ..
            },
            InteractionResponse::ClaudeMode {
                ..
            },
        ) if method == "claude/can_use_tool" => Ok(()),
        | (
            Event::Permission {
                method,
                details,
            },
            InteractionResponse::ClaudeSuggestion {
                index,
            },
        ) if method == "claude/can_use_tool"
            && details["suppress_always_allow_rule"] != true
            && details["permission_suggestions"]
                .as_array()
                .is_some_and(|suggestions| suggestions.get(*index).is_some()) =>
            Ok(()),
        | (_, InteractionResponse::Cancelled)
        | (
            Event::Permission {
                ..
            },
            InteractionResponse::Approval {
                ..
            },
        ) => Ok(()),
        | (
            Event::UserInput {
                questions,
            },
            InteractionResponse::Answers {
                answers,
            },
        ) => {
            if answers.len() != questions.len() || questions.is_empty() || questions.len() > 16 {
                return Err("Answer every requested question");
            }
            for question in questions {
                let values = answers.get(&question.id).ok_or("Missing question ID")?;
                if values.is_empty()
                    || values.len() > 32
                    || (!question.multi_select && values.len() != 1)
                    || values.iter().any(|value| value.trim().is_empty() || value.len() > 16_384)
                {
                    return Err("Invalid answer value or selection count");
                }
            }
            Ok(())
        },
        | _ => Err("Response type does not match the pending request"),
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;
    use crate::{ClaudePermissionMode,
                Event,
                InteractionResponse};

    #[test]
    fn given_titled_multiple_choices_only_offered_strings_are_accepted() {
        use crate::ElicitationAction;
        use serde_json::json;
        for keyword in ["anyOf", "oneOf"] {
            let event = Event::Elicitation {
                server_name: "fixture".into(),
                message: "Choose tags".into(),
                url: None,
                schema: Some(json!({"type":"object","properties":{"tags":{"type":"array","items":{
                    keyword:[{"const":"a","title":"Alpha"},{"const":"b","title":"Beta"}]
                }}},"required":["tags"]})),
            };
            for (tags, valid) in [(json!(["a", "b"]), true), (json!(["unknown"]), false), (json!([true]), false)] {
                let response = InteractionResponse::Elicitation {
                    action: ElicitationAction::Accept,
                    content: Some(json!({"tags":tags})),
                };
                assert_eq!(validate_response(&event, &response).is_ok(), valid);
            }
        }
    }

    #[test]
    fn given_an_mcp_form_only_matching_bounded_content_can_be_accepted() {
        use crate::ElicitationAction::*;
        use serde_json::json;
        let event = Event::Elicitation {
            server_name: "fixture".into(),
            message: "Configure".into(),
            url: None,
            schema: Some(json!({"type":"object","properties":{
                "name":{"type":"string","minLength":1,"maxLength":8},
                "count":{"type":"integer","minimum":1,"maximum":3},
                "enabled":{"type":"boolean"},
                "mode":{"type":"string","oneOf":[{"const":"fast","title":"Fast"},{"const":"slow","title":"Slow"}]},
                "tags":{"type":"array","items":{"type":"string","enum":["a","b"]},"minItems":1,"maxItems":2}
            },"required":["name","count"]})),
        };
        for content in [
            json!({"name":"🦀","count":2}),
            json!({"name":"fixture","count":3,"enabled":false,"mode":"fast","tags":["a","b"]}),
        ] {
            assert!(
                validate_response(
                    &event,
                    &InteractionResponse::Elicitation {
                        action: Accept,
                        content: Some(content)
                    }
                )
                .is_ok()
            );
        }
        for content in [
            json!({}),
            json!({"name":"","count":2}),
            json!({"name":"too long name","count":2}),
            json!({"name":"ok","count":"2"}),
            json!({"name":"ok","count":1.5}),
            json!({"name":"ok","count":4}),
            json!({"name":"ok","count":2,"enabled":"false"}),
            json!({"name":"ok","count":2,"extra":true}),
            json!({"name":"ok","count":2,"mode":"unknown"}),
            json!({"name":"ok","count":2,"tags":["a","a"]}),
            json!({"name":"ok","count":2,"tags":["c"]}),
            json!({"name":"ok","count":2,"tags":[]}),
        ] {
            assert!(
                validate_response(
                    &event,
                    &InteractionResponse::Elicitation {
                        action: Accept,
                        content: Some(content.clone())
                    }
                )
                .is_err(),
                "{content}"
            );
        }
        assert!(
            validate_response(
                &event,
                &InteractionResponse::Approval {
                    allow: true
                }
            )
            .is_err()
        );
        for action in [Decline, Cancel] {
            assert!(
                validate_response(
                    &event,
                    &InteractionResponse::Elicitation {
                        action,
                        content: None
                    }
                )
                .is_ok()
            );
            assert!(
                validate_response(
                    &event,
                    &InteractionResponse::Elicitation {
                        action,
                        content: Some(json!({"name":"unexpected"}))
                    }
                )
                .is_err()
            );
        }
        assert!(validate_response(&event, &InteractionResponse::Cancelled).is_ok());
    }

    #[test]
    fn claude_permissions_cannot_be_applied_to_other_providers_or_unoffered_rules() {
        let mut event = Event::Permission {
            method: "claude/can_use_tool".into(),
            details: serde_json::json!({"permission_suggestions":[{"type":"setMode","mode":"acceptEdits","destination":"session"}]}),
        };
        for mode in ClaudePermissionMode::ALL {
            assert!(
                validate_response(
                    &event,
                    &InteractionResponse::ClaudeMode {
                        mode
                    }
                )
                .is_ok()
            );
        }
        let choice = InteractionResponse::ClaudeSuggestion {
            index: 0,
        };
        assert!(validate_response(&event, &choice).is_ok());
        assert!(
            validate_response(
                &event,
                &InteractionResponse::ClaudeSuggestion {
                    index: 1
                }
            )
            .is_err()
        );
        if let Event::Permission {
            details, ..
        } = &mut event
        {
            details["suppress_always_allow_rule"] = serde_json::json!(true);
        }
        assert!(validate_response(&event, &choice).is_err());
        if let Event::Permission {
            method, ..
        } = &mut event
        {
            *method = "item/commandExecution/requestApproval".into();
        }
        assert!(
            validate_response(
                &event,
                &InteractionResponse::ClaudeMode {
                    mode: ClaudePermissionMode::BypassPermissions
                }
            )
            .is_err()
        );
        assert!(validate_response(&event, &choice).is_err());
        assert!(serde_json::from_value::<InteractionResponse>(serde_json::json!({"type":"claude_mode","mode":"unknown"})).is_err());
    }
}
