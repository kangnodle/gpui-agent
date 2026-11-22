#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition<S, E> {
    pub state: S,
    pub effects: Vec<E>,
}

impl<S, E> Transition<S, E> {
    pub fn pure(state: S) -> Self {
        Self {
            state,
            effects: Vec::new(),
        }
    }

    pub fn with_effect(state: S, effect: E) -> Self {
        Self {
            state,
            effects: vec![effect],
        }
    }

    pub fn with_effects(state: S, effects: Vec<E>) -> Self {
        Self {
            state,
            effects,
        }
    }

    pub fn map_state<T>(self, transform: impl FnOnce(S) -> T) -> Transition<T, E> {
        Transition {
            state: transform(self.state),
            effects: self.effects,
        }
    }

    pub fn map_effects<F>(self, transform: impl FnMut(E) -> F) -> Transition<S, F> {
        Transition {
            state: self.state,
            effects: self.effects.into_iter().map(transform).collect(),
        }
    }

    pub fn and_then<T>(mut self, next: impl FnOnce(S) -> Transition<T, E>) -> Transition<T, E> {
        let following = next(self.state);
        self.effects.extend(following.effects);
        Transition {
            state: following.state,
            effects: self.effects,
        }
    }
}

pub fn fold_transitions<S, I, E>(initial: S, inputs: impl IntoIterator<Item = I>, mut step: impl FnMut(S, I) -> Transition<S, E>) -> Transition<S, E> {
    inputs
        .into_iter()
        .fold(Transition::pure(initial), |acc, input| acc.and_then(|state| step(state, input)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_transitions_accumulates_state_and_effects_purely() {
        let result = fold_transitions(0i32, [1, 2, 3], |state, delta| {
            let next = state + delta;
            Transition::with_effect(next, format!("+{delta}=>{next}"))
        });
        assert_eq!(result.state, 6);
        assert_eq!(result.effects, vec!["+1=>1", "+2=>3", "+3=>6"]);
    }
}
