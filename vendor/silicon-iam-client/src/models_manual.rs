//! Contract shapes a generator cannot express.
//!
//! Kept beside the generated types and re-exported from [`crate::models`], so
//! a caller never has to know which of the two a type came from.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What a trust rule applies to.
///
/// The contract models this as one object whose `kind` decides which
/// identifier is present. An enum says the same thing in a way that cannot be
/// built wrong -- there is no way to name a tag rule and then supply a
/// membership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrustSelector {
    /// Everyone carrying one tag.
    Tag {
        /// The tag.
        tag_id: Uuid,
    },
    /// One specific membership.
    Membership {
        /// The membership.
        membership_id: String,
    },
}

impl TrustSelector {
    /// Selects everyone carrying a tag.
    #[must_use]
    pub const fn tag(tag_id: Uuid) -> Self {
        Self::Tag { tag_id }
    }

    /// Selects one membership.
    #[must_use]
    pub fn membership(membership_id: String) -> Self {
        Self::Membership { membership_id }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::TrustSelector;

    #[test]
    fn a_selector_round_trips_through_the_contract_shape() {
        let tag = TrustSelector::tag(Uuid::from_u128(7));
        let Ok(encoded) = serde_json::to_value(&tag) else {
            panic!("a selector must serialize");
        };
        assert_eq!(encoded["kind"], "tag");
        assert!(encoded.get("membership_id").is_none());

        let Ok(decoded) = serde_json::from_value::<TrustSelector>(encoded) else {
            panic!("a selector must round trip");
        };
        assert_eq!(decoded, tag);
    }

    #[test]
    fn a_membership_selector_names_only_its_membership() {
        let Ok(encoded) =
            serde_json::to_value(TrustSelector::membership("helper:tos[tos]".to_owned()))
        else {
            panic!("a selector must serialize");
        };
        assert_eq!(encoded["kind"], "membership");
        assert!(encoded.get("tag_id").is_none());
    }
}
