//! What a sync should do, from three facts: what is on this PC now, what this
//! PC last synced, and what the account holds.

/// What this PC last agreed with the account on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Base {
    /// The cloud version this PC last uploaded or restored.
    pub version: Option<String>,
    /// The saves' [`crate::bundle::content_hash`] at that moment.
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Both sides are where they were.
    Nothing,
    /// Only this PC changed: back it up.
    Upload,
    /// Only the account changed (another PC played): put its saves here.
    FastForward,
    /// Both changed since they last agreed: the player picks.
    Conflict,
}

/// `local` is the content hash of the saves on this PC (None when there are
/// none), `cloud` the id of the account's newest version (None when it has
/// none) and `cloud_hash` that version's content hash, when known.
pub fn decide(local: Option<&str>, base: &Base, cloud: Option<&str>, cloud_hash: Option<&str>) -> Action {
    let local_changed = match (local, base.hash.as_deref()) {
        (None, _) => false,
        (Some(l), Some(b)) => l != b,
        (Some(_), None) => true,
    };
    match cloud {
        None => {
            if local_changed {
                Action::Upload
            } else {
                Action::Nothing
            }
        }
        Some(head) if base.version.as_deref() == Some(head) => {
            if local_changed {
                Action::Upload
            } else {
                Action::Nothing
            }
        }
        Some(_) => {
            // The account moved on. The same saves on both sides need nothing
            // but remembering; none here, or unchanged here, take the account's.
            if local.is_some() && local == cloud_hash {
                Action::Nothing
            } else if local.is_none() || !local_changed {
                Action::FastForward
            } else {
                Action::Conflict
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(v: Option<&str>, h: Option<&str>) -> Base {
        Base { version: v.map(Into::into), hash: h.map(Into::into) }
    }

    #[test]
    fn every_branch() {
        // Nothing anywhere yet.
        assert_eq!(decide(None, &Base::default(), None, None), Action::Nothing);
        // First saves on this PC, nothing on the account.
        assert_eq!(decide(Some("a"), &Base::default(), None, None), Action::Upload);
        // In step, then played here.
        assert_eq!(decide(Some("a"), &base(Some("v1"), Some("a")), Some("v1"), Some("a")), Action::Nothing);
        assert_eq!(decide(Some("b"), &base(Some("v1"), Some("a")), Some("v1"), Some("a")), Action::Upload);
        // Another PC played; this one did not: take it.
        assert_eq!(decide(Some("a"), &base(Some("v1"), Some("a")), Some("v2"), Some("c")), Action::FastForward);
        // A fresh PC with no saves: take the account's.
        assert_eq!(decide(None, &Base::default(), Some("v2"), Some("c")), Action::FastForward);
        // Both played since they last agreed.
        assert_eq!(decide(Some("b"), &base(Some("v1"), Some("a")), Some("v2"), Some("c")), Action::Conflict);
        // A PC that already had saves before it ever synced, and the account has others.
        assert_eq!(decide(Some("b"), &Base::default(), Some("v2"), Some("c")), Action::Conflict);
        // ...unless they are the very same saves.
        assert_eq!(decide(Some("c"), &Base::default(), Some("v2"), Some("c")), Action::Nothing);
        // The account's saves were removed on kryo.to: this PC's are backed up again.
        assert_eq!(decide(Some("a"), &base(Some("v1"), Some("a")), None, None), Action::Nothing);
        assert_eq!(decide(Some("b"), &base(Some("v1"), Some("a")), None, None), Action::Upload);
    }
}
