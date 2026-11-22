use gpui_agent_presentation::RightPanels;

#[test]
fn given_a_saved_panel_width_when_the_window_shrinks_then_the_chat_keeps_its_minimum_and_the_preferred_width_survives() {
    let panels = RightPanels {
        width: 600.,
        ..Default::default()
    };
    assert_eq!(panels.width_for(844.), 364.);
    assert_eq!(panels.width_for(1500.), 600.);
    assert_eq!(panels.width, 600.);
}

#[test]
fn given_invalid_saved_dimensions_when_normalized_then_defaults_and_safe_split_limits_are_used() {
    let panels = RightPanels {
        width: f32::INFINITY,
        work_ratio: f32::NAN,
        ..Default::default()
    }
    .normalized();
    assert_eq!(panels.width, 320.);
    assert_eq!(panels.work_ratio, 0.5);
    assert_eq!(
        RightPanels {
            work_ratio: -1.,
            ..panels
        }
        .normalized()
        .work_ratio,
        0.2
    );
    assert_eq!(
        RightPanels {
            work_ratio: 2.,
            ..panels
        }
        .normalized()
        .work_ratio,
        0.8
    );
}
