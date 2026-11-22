/// Shared right-column preferences. Viewport constraints never overwrite
/// intent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RightPanels {
    pub work_open: bool,
    pub reference_open: bool,
    pub width: f32,
    pub work_ratio: f32,
    /// Width of the PR/Git pane; `None` keeps each panel's default.
    pub tool_width: Option<f32>,
}

impl Default for RightPanels {
    fn default() -> Self {
        Self {
            work_open: true,
            reference_open: false,
            width: 320.,
            work_ratio: 0.5,
            tool_width: None,
        }
    }
}

impl RightPanels {
    pub fn normalized(mut self) -> Self {
        if !self.width.is_finite() || self.width < 280. {
            self.width = 320.;
        }
        self.work_ratio = if self.work_ratio.is_finite() { self.work_ratio.clamp(0.2, 0.8) } else { 0.5 };
        self.tool_width = self.tool_width.filter(|width| width.is_finite()).map(|width| width.max(280.));
        self
    }

    pub fn width_for(&self, available: f32) -> f32 { self.width.clamp(280., (available - 480.).max(280.)) }

    /// Width of the PR/Git pane, keeping 480px for the conversation.
    pub fn tool_width_for(&self, default: f32, available: f32) -> f32 { self.tool_width.unwrap_or(default).clamp(280., (available - 480.).max(280.)) }
}

#[cfg(test)]
mod tests {
    use super::RightPanels;

    #[test]
    fn given_no_saved_tool_width_when_laid_out_then_the_panel_default_is_used_and_clamped() {
        let panels = RightPanels::default();
        assert_eq!(panels.tool_width_for(360., 1400.), 360.);
        let wide = RightPanels {
            tool_width: Some(2000.),
            ..panels
        };
        assert_eq!(wide.tool_width_for(360., 1400.), 920.);
        let narrow = RightPanels {
            tool_width: Some(10.),
            ..panels
        }
        .normalized();
        assert_eq!(narrow.tool_width_for(360., 1400.), 280.);
    }
}
