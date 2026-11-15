//! What the UI shows for neighborhoods and direct messages.
//!
//! Like `conflict.rs` for households, these are folds over envelopes that give
//! the same result whatever order the envelopes arrived in.

use std::collections::HashMap;

use beacon_protocol::{
    AlertCategory, CheckInState, Envelope, EnvelopeId, NeighborhoodId, Payload, PersonId,
    UnixMillis,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostKind {
    Alert {
        category: AlertCategory,
        text: String,
        area: Option<String>,
    },
    Offer {
        text: String,
    },
    Need {
        text: String,
    },
    HouseholdStatus {
        household_name: String,
        all_safe: bool,
        text: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub envelope_id: EnvelopeId,
    pub author: PersonId,
    pub created_at: UnixMillis,
    pub kind: PostKind,
}

/// Posts for one neighborhood.
#[derive(Debug, Default)]
pub struct NeighborhoodFeed {
    posts: HashMap<EnvelopeId, Post>,
    /// Withdrawn post id -> who withdrew it. Only the original author's
    /// withdrawal counts; checked at read time so arrival order doesn't matter.
    withdrawals: HashMap<EnvelopeId, Vec<PersonId>>,
}

impl NeighborhoodFeed {
    pub fn apply(&mut self, env: &Envelope) -> bool {
        let kind = match &env.payload {
            Payload::Alert {
                category,
                text,
                area,
            } => PostKind::Alert {
                category: *category,
                text: text.clone(),
                area: area.clone(),
            },
            Payload::Offer { text } => PostKind::Offer { text: text.clone() },
            Payload::Need { text } => PostKind::Need { text: text.clone() },
            Payload::HouseholdStatus {
                household_name,
                all_safe,
                text,
            } => PostKind::HouseholdStatus {
                household_name: household_name.clone(),
                all_safe: *all_safe,
                text: text.clone(),
            },
            Payload::Withdraw { envelope_id } => {
                self.withdrawals
                    .entry(*envelope_id)
                    .or_default()
                    .push(env.author);
                return true;
            }
            _ => return false,
        };
        self.posts
            .insert(
                env.id,
                Post {
                    envelope_id: env.id,
                    author: env.author,
                    created_at: env.created_at,
                    kind,
                },
            )
            .is_none()
    }

    fn is_withdrawn(&self, post: &Post) -> bool {
        self.withdrawals
            .get(&post.envelope_id)
            .is_some_and(|by| by.contains(&post.author))
    }

    /// Visible posts, newest first. `visible_author` lets the caller hide
    /// removed and blocked people.
    pub fn posts(&self, visible_author: impl Fn(&PersonId) -> bool) -> Vec<&Post> {
        let mut out: Vec<&Post> = self
            .posts
            .values()
            .filter(|p| !self.is_withdrawn(p) && visible_author(&p.author))
            .collect();
        out.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then(b.envelope_id.cmp(&a.envelope_id))
        });
        out
    }

    pub fn prune(&mut self, now: UnixMillis, ttl_ms: u64) {
        self.posts
            .retain(|_, p| p.created_at.saturating_add(ttl_ms) > now);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectKind {
    Message {
        text: String,
    },
    WellnessRequest {
        text: Option<String>,
    },
    WellnessReply {
        state: CheckInState,
        text: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectMessage {
    pub envelope_id: EnvelopeId,
    pub from: PersonId,
    pub to: PersonId,
    pub created_at: UnixMillis,
    pub kind: DirectKind,
}

/// Direct messages sent and received by this device's person.
#[derive(Debug, Default)]
pub struct Inbox {
    messages: HashMap<EnvelopeId, DirectMessage>,
}

impl Inbox {
    pub fn apply(&mut self, env: &Envelope, to: PersonId) -> bool {
        let kind = match &env.payload {
            Payload::Message { text } => DirectKind::Message { text: text.clone() },
            Payload::WellnessRequest { text } => DirectKind::WellnessRequest { text: text.clone() },
            Payload::WellnessReply { state, text } => DirectKind::WellnessReply {
                state: *state,
                text: text.clone(),
            },
            _ => return false,
        };
        self.messages
            .insert(
                env.id,
                DirectMessage {
                    envelope_id: env.id,
                    from: env.author,
                    to,
                    created_at: env.created_at,
                    kind,
                },
            )
            .is_none()
    }

    /// Everything, newest first.
    pub fn all(&self) -> Vec<&DirectMessage> {
        let mut out: Vec<_> = self.messages.values().collect();
        out.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then(b.envelope_id.cmp(&a.envelope_id))
        });
        out
    }

    /// One conversation, oldest first, as a chat view wants it.
    pub fn conversation(&self, me: PersonId, other: PersonId) -> Vec<&DirectMessage> {
        let mut out: Vec<_> = self
            .messages
            .values()
            .filter(|m| (m.from == me && m.to == other) || (m.from == other && m.to == me))
            .collect();
        out.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.envelope_id.cmp(&b.envelope_id))
        });
        out
    }
}

/// Scope-tagged view of neighborhood activity, used when one person belongs
/// to several neighborhoods.
pub type Feeds = HashMap<NeighborhoodId, NeighborhoodFeed>;

#[cfg(test)]
mod tests {
    use super::*;
    use beacon_protocol::{Audience, DeviceId, NewEnvelope};

    fn env(author: PersonId, at: UnixMillis, payload: Payload) -> Envelope {
        Envelope::new(NewEnvelope {
            audience: Audience::Neighborhood {
                neighborhood_id: NeighborhoodId::new(),
            },
            author,
            origin_device: DeviceId::new(),
            lamport: 1,
            now: at,
            payload,
        })
    }

    #[test]
    fn posts_are_newest_first_and_authors_can_withdraw_their_own() {
        let (a, b) = (PersonId::new(), PersonId::new());
        let mut feed = NeighborhoodFeed::default();
        let offer = env(
            a,
            1,
            Payload::Offer {
                text: "Generator at #12".into(),
            },
        );
        let need = env(
            b,
            2,
            Payload::Need {
                text: "Need ice".into(),
            },
        );
        feed.apply(&offer);
        feed.apply(&need);
        assert_eq!(feed.posts(|_| true)[0].envelope_id, need.id);

        // B can't withdraw A's post, but A can, even if the withdrawal
        // arrives first.
        let mut fresh = NeighborhoodFeed::default();
        fresh.apply(&env(
            b,
            3,
            Payload::Withdraw {
                envelope_id: offer.id,
            },
        ));
        fresh.apply(&env(
            a,
            4,
            Payload::Withdraw {
                envelope_id: offer.id,
            },
        ));
        fresh.apply(&offer);
        assert!(fresh.posts(|_| true).is_empty());

        assert_eq!(feed.posts(|p| *p != b).len(), 1);
    }

    #[test]
    fn inbox_threads_conversations() {
        let (me, dana) = (PersonId::new(), PersonId::new());
        let mut inbox = Inbox::default();
        inbox.apply(&env(dana, 1, Payload::WellnessRequest { text: None }), me);
        inbox.apply(
            &env(
                me,
                2,
                Payload::WellnessReply {
                    state: CheckInState::Safe,
                    text: None,
                },
            ),
            dana,
        );
        let convo = inbox.conversation(me, dana);
        assert_eq!(convo.len(), 2);
        assert_eq!(convo[0].from, dana);
    }
}
