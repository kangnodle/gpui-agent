#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenObservation {
    pub revision: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComputerInputTarget {
    Click { x: u32, y: u32 },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputerLeaseState {
    Granted {
        job_id: i64,
    },
    Bound {
        job_id: i64,
        execution_id: u64,
    },
    Observed {
        job_id: i64,
        execution_id: u64,
        observation: ScreenObservation,
    },
}

impl ComputerLeaseState {
    pub fn granted(job_id: i64) -> Self {
        Self::Granted {
            job_id,
        }
    }

    pub fn job_id(&self) -> i64 {
        match self {
            | Self::Granted {
                job_id,
            }
            | Self::Bound {
                job_id, ..
            }
            | Self::Observed {
                job_id, ..
            } => *job_id,
        }
    }

    pub fn execution_id(&self) -> Option<u64> {
        match self {
            | Self::Granted {
                ..
            } => None,
            | Self::Bound {
                execution_id, ..
            }
            | Self::Observed {
                execution_id, ..
            } => Some(*execution_id),
        }
    }

    pub fn observation(&self) -> Option<&ScreenObservation> {
        match self {
            | Self::Observed {
                observation, ..
            } => Some(observation),
            | _ => None,
        }
    }

    pub fn authorize(&self, job_id: i64, execution_id: u64) -> Result<(), &'static str> {
        if self.job_id() != job_id {
            return Err("Computer grant belongs to another job");
        }
        if self.execution_id().is_some_and(|owner| owner != execution_id) {
            return Err("Computer grant belongs to another execution session");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComputerLeaseInput {
    Bind { execution_id: u64 },
    Observe(ScreenObservation),
    Consume { revision: String, target: ComputerInputTarget },
}

pub fn transition_computer_lease(state: ComputerLeaseState, input: ComputerLeaseInput) -> Result<ComputerLeaseState, &'static str> {
    match (state, input) {
        | (
            state,
            ComputerLeaseInput::Bind {
                execution_id,
            },
        ) => Ok(ComputerLeaseState::Bound {
            job_id: state.job_id(),
            execution_id,
        }),
        | (
            ComputerLeaseState::Bound {
                job_id,
                execution_id,
            }
            | ComputerLeaseState::Observed {
                job_id,
                execution_id,
                ..
            },
            ComputerLeaseInput::Observe(observation),
        ) => Ok(ComputerLeaseState::Observed {
            job_id,
            execution_id,
            observation,
        }),
        | (
            ComputerLeaseState::Granted {
                ..
            },
            ComputerLeaseInput::Observe(_),
        ) => Err("Bind a computer execution session before recording observations"),
        | (
            ComputerLeaseState::Observed {
                job_id,
                execution_id,
                observation,
            },
            ComputerLeaseInput::Consume {
                revision,
                target,
            },
        ) => {
            if observation.revision != revision {
                return Err("Stale computer capture revision; capture again before input");
            }
            if matches!(
                target,
                ComputerInputTarget::Click { x, y } if x >= observation.width || y >= observation.height
            ) {
                return Err("Click exceeds the captured computer screen");
            }
            Ok(ComputerLeaseState::Bound {
                job_id,
                execution_id,
            })
        },
        | (
            ComputerLeaseState::Granted {
                ..
            }
            | ComputerLeaseState::Bound {
                ..
            },
            ComputerLeaseInput::Consume {
                ..
            },
        ) => Err("Capture the computer before input"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computer_lease_adt_enforces_bind_observe_consume_sequence() {
        let granted = ComputerLeaseState::granted(42);
        assert!(granted.authorize(42, 7).is_ok());
        assert!(granted.authorize(99, 7).is_err());

        let bound = transition_computer_lease(
            granted,
            ComputerLeaseInput::Bind {
                execution_id: 7,
            },
        )
        .unwrap();
        assert!(bound.authorize(42, 7).is_ok());
        assert!(bound.authorize(42, 8).is_err());
        assert!(
            transition_computer_lease(
                bound.clone(),
                ComputerLeaseInput::Consume {
                    revision: "r1".into(),
                    target: ComputerInputTarget::Other,
                }
            )
            .is_err()
        );

        let observed = transition_computer_lease(
            bound,
            ComputerLeaseInput::Observe(ScreenObservation {
                revision: "r1".into(),
                width: 100,
                height: 50,
            }),
        )
        .unwrap();
        assert!(
            transition_computer_lease(
                observed.clone(),
                ComputerLeaseInput::Consume {
                    revision: "stale".into(),
                    target: ComputerInputTarget::Other,
                }
            )
            .is_err()
        );
        assert!(
            transition_computer_lease(
                observed.clone(),
                ComputerLeaseInput::Consume {
                    revision: "r1".into(),
                    target: ComputerInputTarget::Click {
                        x: 100,
                        y: 10,
                    },
                }
            )
            .is_err()
        );

        let consumed = transition_computer_lease(
            observed,
            ComputerLeaseInput::Consume {
                revision: "r1".into(),
                target: ComputerInputTarget::Click {
                    x: 99,
                    y: 49,
                },
            },
        )
        .unwrap();
        assert_eq!(
            consumed,
            ComputerLeaseState::Bound {
                job_id: 42,
                execution_id: 7,
            }
        );
    }
}
