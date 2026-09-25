//! Enabled state of a winning item.

use loadout_model::Mode;

/// Why an item ended up enabled or disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnabledReason {
    /// `mode: required` or `locked: true`; toggles are ignored.
    Required,
    /// The item's default for its mode.
    Default,
    /// The user's toggle.
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EnabledState {
    pub enabled: bool,
    pub reason: EnabledReason,
    /// The user tried to disable an item that cannot be disabled.
    pub ignored_toggle: bool,
}

/// Computes whether an item is enabled. A missing mode means `default-on`.
pub fn enabled_state(mode: Option<Mode>, locked: bool, toggle: Option<bool>) -> EnabledState {
    let mode = mode.unwrap_or(Mode::DefaultOn);
    if mode == Mode::Required || locked {
        return EnabledState {
            enabled: true,
            reason: EnabledReason::Required,
            ignored_toggle: toggle == Some(false),
        };
    }
    match toggle {
        Some(on) => EnabledState {
            enabled: on,
            reason: EnabledReason::Toggle,
            ignored_toggle: false,
        },
        None => EnabledState {
            enabled: mode == Mode::DefaultOn,
            reason: EnabledReason::Default,
            ignored_toggle: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_ignores_toggle_off_and_flags_it() {
        let s = enabled_state(Some(Mode::Required), false, Some(false));
        assert!(s.enabled);
        assert_eq!(s.reason, EnabledReason::Required);
        assert!(s.ignored_toggle);
    }

    #[test]
    fn locked_behaves_like_required() {
        let s = enabled_state(Some(Mode::DefaultOff), true, Some(false));
        assert!(s.enabled && s.ignored_toggle);
        let s = enabled_state(Some(Mode::DefaultOff), true, Some(true));
        assert!(s.enabled && !s.ignored_toggle);
    }

    #[test]
    fn default_on_and_off() {
        assert!(enabled_state(Some(Mode::DefaultOn), false, None).enabled);
        assert!(enabled_state(None, false, None).enabled);
        assert!(!enabled_state(Some(Mode::DefaultOn), false, Some(false)).enabled);
        assert!(!enabled_state(Some(Mode::DefaultOff), false, None).enabled);
        let s = enabled_state(Some(Mode::DefaultOff), false, Some(true));
        assert!(s.enabled);
        assert_eq!(s.reason, EnabledReason::Toggle);
    }
}
