//! The snapshot the app renders, as JSON.
//!
//! Shapes match what the TypeScript mirror builds today
//! (apps/mobile/src/sync/householdView.ts and neighborhoodView.ts), so the
//! screens don't change when the mirror is retired: protocol types keep their
//! snake_case wire form, view wrappers are camelCase.

use std::collections::BTreeMap;

use beacon_sync_core::feed::{DirectKind, DirectMessage, Post, PostKind};
use beacon_sync_core::neighborhood::{NeighborhoodState, INVITES_PER_DAY};
use beacon_sync_core::protocol::{
    AlertCategory, CheckIn, CheckInState, EnvelopeId, Household, MembershipCert, Neighborhood,
    PersonId, RendezvousPoint, UnixMillis,
};
use beacon_sync_core::store::EnvelopeStore;
use beacon_sync_core::transport::Transport;
use beacon_sync_core::SyncEngine;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    pub household: Household,
    pub device_id: String,
    pub person_id: PersonId,
    pub view: HouseholdView,
    pub neighborhoods: Vec<NeighborhoodView>,
    /// Oldest first.
    pub inbox: Vec<DirectView>,
    pub blocked: Vec<PersonId>,
    pub relay_enabled: bool,
    pub carrying_for_others: usize,
    pub pending_outbound: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HouseholdView {
    pub check_ins: BTreeMap<PersonId, CheckIn>,
    /// Primary first.
    pub rendezvous: Vec<RendezvousPoint>,
    /// Newest first.
    pub notes: Vec<NoteView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NoteView {
    pub envelope_id: EnvelopeId,
    pub author: PersonId,
    pub text: String,
    pub created_at: UnixMillis,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NeighborhoodView {
    pub info: Neighborhood,
    /// Sorted by name.
    pub members: Vec<MembershipCert>,
    pub stewards: Vec<PersonId>,
    pub is_founder: bool,
    pub is_steward: bool,
    pub removed_me: bool,
    pub over_soft_cap: bool,
    pub invites_left_today: usize,
    /// Newest first; current members only, minus blocked people.
    pub feed: Vec<PostView>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PostView {
    pub envelope_id: EnvelopeId,
    pub author: PersonId,
    pub created_at: UnixMillis,
    #[serde(flatten)]
    pub kind: PostKindView,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum PostKindView {
    Alert {
        category: AlertCategory,
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
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
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DirectView {
    pub envelope_id: EnvelopeId,
    pub from: PersonId,
    pub to: PersonId,
    pub created_at: UnixMillis,
    #[serde(flatten)]
    pub kind: DirectKindView,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum DirectKindView {
    Message {
        text: String,
    },
    WellnessRequest {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
    WellnessReply {
        state: CheckInState,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

pub(crate) fn snapshot<S: EnvelopeStore, T: Transport>(
    engine: &SyncEngine<S, T>,
    blocked: Vec<PersonId>,
    now: UnixMillis,
) -> Snapshot {
    let me = engine.me();
    let state = engine.state();
    Snapshot {
        household: engine.household().clone(),
        device_id: engine.device_id().to_string(),
        person_id: me,
        view: HouseholdView {
            check_ins: state
                .check_ins()
                .into_iter()
                .map(|c| (c.person_id, c.clone()))
                .collect(),
            rendezvous: state.rendezvous_points().into_iter().cloned().collect(),
            notes: state
                .notes()
                .iter()
                .map(|n| NoteView {
                    envelope_id: n.envelope_id,
                    author: n.author,
                    text: n.text.clone(),
                    created_at: n.created_at,
                })
                .collect(),
        },
        neighborhoods: engine
            .neighborhoods()
            .into_iter()
            .map(|n| neighborhood_view(engine, n, me, now))
            .collect(),
        inbox: engine
            .inbox()
            .all()
            .into_iter()
            .filter(|m| !engine.is_blocked(&m.from))
            .map(direct_view)
            .collect(),
        blocked,
        relay_enabled: engine.relay_enabled(),
        carrying_for_others: engine.carrying_for_others(),
        pending_outbound: engine.pending_outbound(),
    }
}

fn neighborhood_view<S: EnvelopeStore, T: Transport>(
    engine: &SyncEngine<S, T>,
    n: &NeighborhoodState,
    me: PersonId,
    now: UnixMillis,
) -> NeighborhoodView {
    let mut members: Vec<MembershipCert> = n.members().into_iter().cloned().collect();
    members.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    let stewards = members
        .iter()
        .map(|c| c.member)
        .filter(|p| n.is_steward(p))
        .collect();
    NeighborhoodView {
        info: n.info.clone(),
        stewards,
        members,
        is_founder: n.is_founder(&me),
        is_steward: n.is_steward(&me),
        removed_me: n.is_removed(&me),
        over_soft_cap: n.over_soft_cap(),
        invites_left_today: INVITES_PER_DAY.saturating_sub(n.invites_in_last_day(&me, now)),
        feed: engine.feed(&n.info.id).into_iter().map(post_view).collect(),
    }
}

fn post_view(p: &Post) -> PostView {
    PostView {
        envelope_id: p.envelope_id,
        author: p.author,
        created_at: p.created_at,
        kind: match &p.kind {
            PostKind::Alert {
                category,
                text,
                area,
            } => PostKindView::Alert {
                category: *category,
                text: text.clone(),
                area: area.clone(),
            },
            PostKind::Offer { text } => PostKindView::Offer { text: text.clone() },
            PostKind::Need { text } => PostKindView::Need { text: text.clone() },
            PostKind::HouseholdStatus {
                household_name,
                all_safe,
                text,
            } => PostKindView::HouseholdStatus {
                household_name: household_name.clone(),
                all_safe: *all_safe,
                text: text.clone(),
            },
        },
    }
}

fn direct_view(m: &DirectMessage) -> DirectView {
    DirectView {
        envelope_id: m.envelope_id,
        from: m.from,
        to: m.to,
        created_at: m.created_at,
        kind: match &m.kind {
            DirectKind::Message { text } => DirectKindView::Message { text: text.clone() },
            DirectKind::WellnessRequest { text } => {
                DirectKindView::WellnessRequest { text: text.clone() }
            }
            DirectKind::WellnessReply { state, text } => DirectKindView::WellnessReply {
                state: *state,
                text: text.clone(),
            },
        },
    }
}
