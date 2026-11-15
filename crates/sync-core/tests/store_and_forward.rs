//! End-to-end scenarios over the mock transport: households, sealed relay,
//! neighborhoods grown by invite chains, and direct messages.
//!
//! Each test models phones as engines joined to one `MockNetwork` and moves
//! people in and out of range explicitly.

use beacon_sync_core::crypto::SymmetricKey;
use beacon_sync_core::protocol::{
    AlertCategory, CheckIn, CheckInState, DeviceId, Household, HouseholdId, NeighborhoodId,
    Payload, PersonId, PersonRole, RemovalReason, RendezvousId, RendezvousPoint, UnixMillis,
};
use beacon_sync_core::sqlite_store::SqliteStore;
use beacon_sync_core::store::{EnvelopeStore, MemoryStore};
use beacon_sync_core::transport::{MockNetwork, MockTransport, PeerAddress, Transport};
use beacon_sync_core::{LocalIdentity, SyncEngine, SyncEngineConfig, SyncError};

const T0: UnixMillis = 1_700_000_000_000;

type Phone = SyncEngine<MemoryStore, MockTransport>;

/// A household whose members each have keys, as after in-person pairing.
struct Family {
    household: Household,
    key: SymmetricKey,
    ids: Vec<PersonId>,
    identities: Vec<Option<LocalIdentity>>,
}

impl Family {
    fn new(name: &str, people: &[&str]) -> Self {
        let identities: Vec<_> = people.iter().map(|n| LocalIdentity::generate(*n)).collect();
        let household = Household {
            id: HouseholdId::new(),
            name: name.into(),
            members: identities
                .iter()
                .map(|i| i.as_person(PersonRole::Adult))
                .collect(),
        };
        Self {
            household,
            key: SymmetricKey::generate(),
            ids: identities.iter().map(|i| i.person_id).collect(),
            identities: identities.into_iter().map(Some).collect(),
        }
    }

    fn id(&self, i: usize) -> PersonId {
        self.ids[i]
    }

    /// Give member `i`'s identity to a new phone.
    fn phone(&mut self, net: &MockNetwork, i: usize) -> Phone {
        let identity = self.identities[i]
            .take()
            .expect("each member gets one phone");
        let name = identity.display_name.to_lowercase();
        let config = SyncEngineConfig::new(
            DeviceId::new(),
            identity,
            self.household.clone(),
            self.key.clone(),
        );
        SyncEngine::new(config, MemoryStore::new(), net.join(name.as_str()), T0)
            .expect("member of household")
    }
}

fn addr(p: &mut Phone) -> PeerAddress {
    p.transport_mut().local_address()
}

fn check_in(person: PersonId, state: CheckInState, at: UnixMillis) -> Payload {
    Payload::CheckIn(CheckIn {
        person_id: person,
        state,
        note: None,
        rendezvous_id: None,
        recorded_at: at,
    })
}

/// Run rounds on every phone until a full pass puts nothing on the air.
fn settle(net: &MockNetwork, phones: &mut [&mut Phone], now: UnixMillis) {
    for _ in 0..12 {
        let before = net.frames_sent();
        for p in phones.iter_mut() {
            p.sync_round(now);
        }
        if net.frames_sent() == before {
            return;
        }
    }
    panic!("phones did not settle");
}

/// Bring two phones into range, settle, separate.
fn meet(net: &MockNetwork, a: &mut Phone, b: &mut Phone, now: UnixMillis) {
    let (x, y) = (addr(a), addr(b));
    net.set_in_range(&x, &y, true);
    settle(net, &mut [a, b], now);
    net.set_in_range(&x, &y, false);
}

// --- households ---

#[test]
fn two_family_phones_in_range_exchange_check_ins() {
    let mut f = Family::new("Okafor", &["Mom", "Dad"]);
    let net = MockNetwork::new();
    let (mom_id, dad_id) = (f.id(0), f.id(1));
    let mut mom = f.phone(&net, 0);
    let mut dad = f.phone(&net, 1);

    mom.record(check_in(mom_id, CheckInState::Safe, T0), T0)
        .unwrap();
    dad.record(check_in(dad_id, CheckInState::OnMyWay, T0), T0)
        .unwrap();
    meet(&net, &mut mom, &mut dad, T0 + 1_000);

    for p in [&mom, &dad] {
        assert_eq!(
            p.state().check_in(&mom_id).unwrap().state,
            CheckInState::Safe
        );
        assert_eq!(
            p.state().check_in(&dad_id).unwrap().state,
            CheckInState::OnMyWay
        );
    }
}

#[test]
fn a_strangers_phone_carries_family_updates_without_reading_them() {
    // The teen is at school; mom is at home. A neighbor from a different
    // household walks from one to the other.
    let mut f = Family::new("Okafor", &["Mom", "Teen"]);
    let mut other = Family::new("Nguyen", &["Linh"]);
    let net = MockNetwork::new();
    let teen_id = f.id(1);
    let mut mom = f.phone(&net, 0);
    let mut teen = f.phone(&net, 1);
    let mut stranger = other.phone(&net, 0);

    teen.record(check_in(teen_id, CheckInState::NeedHelp, T0), T0)
        .unwrap();
    meet(&net, &mut teen, &mut stranger, T0 + 1_000);

    assert_eq!(stranger.carrying_for_others(), 1);
    assert!(
        stranger.state().check_in(&teen_id).is_none(),
        "relay must not read it"
    );
    assert_eq!(stranger.store().readable_len(), 0);

    meet(&net, &mut stranger, &mut mom, T0 + 60_000);
    let seen = mom
        .state()
        .check_in(&teen_id)
        .expect("carried by the neighbor");
    assert_eq!(seen.state, CheckInState::NeedHelp);
}

#[test]
fn relaying_can_be_turned_off() {
    let mut f = Family::new("Okafor", &["Teen"]);
    let mut other = Family::new("Nguyen", &["Linh"]);
    let net = MockNetwork::new();
    let teen_id = f.id(0);
    let mut teen = f.phone(&net, 0);
    let mut stranger = other.phone(&net, 0);
    stranger.set_relay_enabled(false);

    teen.record(check_in(teen_id, CheckInState::Safe, T0), T0)
        .unwrap();
    meet(&net, &mut teen, &mut stranger, T0 + 1_000);
    assert_eq!(stranger.carrying_for_others(), 0);
    assert!(stranger.store().is_empty());
}

#[test]
fn every_family_phone_converges_on_the_same_plan() {
    let mut f = Family::new("Okafor", &["Mom", "Dad"]);
    let net = MockNetwork::new();
    let household_id = f.household.id;
    let mut mom = f.phone(&net, 0);
    let mut dad = f.phone(&net, 1);

    let id = RendezvousId::new();
    let point = |name: &str| {
        Payload::RendezvousUpsert(RendezvousPoint {
            id,
            household_id,
            name: name.into(),
            details: String::new(),
            location: None,
            when_to_go: "If phones are down for 2 hours".into(),
            rank: 1,
        })
    };
    mom.record(point("Park gate"), T0).unwrap();
    meet(&net, &mut mom, &mut dad, T0 + 1_000);
    dad.record(point("Library steps"), T0 + 2_000).unwrap();
    meet(&net, &mut mom, &mut dad, T0 + 3_000);

    for p in [&mom, &dad] {
        assert_eq!(p.state().rendezvous(&id).unwrap().name, "Library steps");
    }
}

#[test]
fn repeat_meetings_do_not_resend_everything() {
    let mut f = Family::new("Okafor", &["Mom", "Dad"]);
    let net = MockNetwork::new();
    let mut mom = f.phone(&net, 0);
    let mut dad = f.phone(&net, 1);
    let (a, b) = (addr(&mut mom), addr(&mut dad));
    net.set_in_range(&a, &b, true);

    for i in 0..10 {
        mom.record(
            Payload::Note {
                text: format!("update {i}"),
            },
            T0 + i,
        )
        .unwrap();
    }
    settle(&net, &mut [&mut mom, &mut dad], T0 + 1_000);
    assert_eq!(dad.state().notes().len(), 10);

    let frames_before = net.frames_sent();
    settle(&net, &mut [&mut mom, &mut dad], T0 + 2_000);
    assert_eq!(net.frames_sent(), frames_before);
}

#[test]
fn radio_off_skips_the_round_and_backs_off() {
    let mut f = Family::new("Okafor", &["Mom"]);
    let net = MockNetwork::new();
    let mut mom = f.phone(&net, 0);
    mom.transport_mut().set_available(false);

    let report = mom.sync_round(T0);
    assert!(report.skipped.is_some());
    assert!(!mom.should_sync(T0 + 1));
}

#[test]
fn state_survives_a_restart_from_the_store() {
    let mut f = Family::new("Okafor", &["Mom"]);
    let net = MockNetwork::new();
    let mom_id = f.id(0);
    let (household, key) = (f.household.clone(), f.key.clone());
    let mut mom = f.phone(&net, 0);
    mom.record(check_in(mom_id, CheckInState::AtRendezvous, T0), T0)
        .unwrap();

    let mut store = MemoryStore::new();
    for entry in mom.store().all(T0).unwrap() {
        store.insert(entry).unwrap();
    }
    // The real app restores keys from the OS keystore; stored envelopes are
    // replayed without re-verification, so fresh keys suffice here.
    let identity = LocalIdentity {
        person_id: mom_id,
        ..LocalIdentity::generate("Mom")
    };
    let config = SyncEngineConfig::new(mom.device_id(), identity, household, key);
    let reopened = SyncEngine::new(config, store, net.join("mom-2"), T0 + 10).unwrap();

    assert_eq!(
        reopened.state().check_in(&mom_id).unwrap().state,
        CheckInState::AtRendezvous
    );
    assert_eq!(reopened.pending_outbound(), 1);
}

#[test]
fn state_survives_an_app_restart_on_sqlite() {
    let f = Family::new("Okafor", &["Mom", "Dad"]);
    let (mom_id, dad_id) = (f.id(0), f.id(1));
    let path = std::env::temp_dir().join(format!("beacon-engine-{}.db", DeviceId::new()));
    let device = DeviceId::new();
    let config = |identity: LocalIdentity| {
        SyncEngineConfig::new(device, identity, f.household.clone(), f.key.clone())
    };
    let net = MockNetwork::new();
    {
        let identity = LocalIdentity {
            person_id: mom_id,
            ..LocalIdentity::generate("Mom")
        };
        let store = SqliteStore::open(&path).unwrap();
        let mut mom = SyncEngine::new(config(identity), store, net.join("mom"), T0).unwrap();
        mom.record(check_in(mom_id, CheckInState::OnMyWay, T0), T0)
            .unwrap();
        mom.record(check_in(dad_id, CheckInState::Safe, T0 + 1), T0 + 1)
            .unwrap();
    } // App killed: the engine and its connection are dropped.

    let identity = LocalIdentity {
        person_id: mom_id,
        ..LocalIdentity::generate("Mom")
    };
    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(store.readable_len().unwrap(), 2);
    let reopened = SyncEngine::new(config(identity), store, net.join("mom-2"), T0 + 10).unwrap();
    assert_eq!(
        reopened.state().check_in(&mom_id).unwrap().state,
        CheckInState::OnMyWay
    );
    assert_eq!(
        reopened.state().check_in(&dad_id).unwrap().state,
        CheckInState::Safe
    );
    assert_eq!(reopened.pending_outbound(), 2);
    drop(reopened);
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }
}

#[test]
fn engine_refuses_a_person_outside_the_household() {
    let f = Family::new("Okafor", &["Mom"]);
    let identity = LocalIdentity::generate("Stranger");
    let config = SyncEngineConfig::new(DeviceId::new(), identity, f.household, f.key);
    let result = SyncEngine::new(config, MemoryStore::new(), MockNetwork::new().join("x"), T0);
    assert!(matches!(result, Err(SyncError::LocalPersonNotMember)));
}

#[test]
fn invalid_local_input_is_rejected_before_it_is_stored() {
    let mut f = Family::new("Okafor", &["Mom"]);
    let net = MockNetwork::new();
    let mut mom = f.phone(&net, 0);
    let err = mom
        .record(Payload::Note { text: "  ".into() }, T0)
        .unwrap_err();
    assert!(matches!(err, SyncError::Invalid(_)));
    assert!(mom.store().is_empty());
}

// --- neighborhoods ---

/// Three households on one street, each with one phone for simplicity.
struct Street {
    net: MockNetwork,
    a: Phone,
    b: Phone,
    c: Phone,
    hood: NeighborhoodId,
}

/// A founds the neighborhood, invites B in person; B invites C in person.
fn street() -> Street {
    let net = MockNetwork::new();
    let mut a = Family::new("Lee", &["Ana"]).phone(&net, 0);
    let mut b = Family::new("Park", &["Ben"]).phone(&net, 0);
    let mut c = Family::new("Diaz", &["Cleo"]).phone(&net, 0);

    let hood = a
        .create_neighborhood("Elm & Oak", "Elm and Oak streets", T0)
        .expect("create neighborhood");
    let invite_b = a
        .invite(hood, &b.invite_request(), T0 + 10)
        .expect("A invites B");
    b.accept_invite(invite_b, T0 + 20).expect("B joins");
    let invite_c = b
        .invite(hood, &c.invite_request(), T0 + 30)
        .expect("B invites C");
    c.accept_invite(invite_c, T0 + 40).expect("C joins");
    Street { net, a, b, c, hood }
}

#[test]
fn invite_chains_let_a_reach_c_through_the_mesh() {
    let mut s = street();
    let (a_id, b_id, c_id) = (s.a.me(), s.b.me(), s.c.me());

    // C learned A from the roster B handed over in person.
    assert!(s.c.neighborhood(&s.hood).unwrap().is_member(&a_id));
    meet(&s.net, &mut s.b, &mut s.c, T0 + 1_000);
    meet(&s.net, &mut s.a, &mut s.b, T0 + 2_000);
    let a_view = s.a.neighborhood(&s.hood).unwrap();
    assert!(a_view.is_member(&c_id));
    assert_eq!(a_view.invite_chain(&c_id), vec![c_id, b_id, a_id]);

    // A posts to the neighborhood and checks on C privately. A and C never
    // meet; B carries both.
    let alert = Payload::Alert {
        category: AlertCategory::Power,
        text: "Transformer blew at Elm & 5th".into(),
        area: Some("Elm St, 400 block".into()),
    };
    s.a.post(s.hood, alert, T0 + 3_000).unwrap();
    let ask = Payload::WellnessRequest {
        text: Some("Are you OK?".into()),
    };
    s.a.send_direct(c_id, ask, T0 + 3_000).unwrap();

    meet(&s.net, &mut s.a, &mut s.b, T0 + 4_000);
    meet(&s.net, &mut s.b, &mut s.c, T0 + 5_000);

    assert_eq!(s.c.feed(&s.hood).len(), 1);
    assert_eq!(s.b.feed(&s.hood).len(), 1);
    let inbox = s.c.inbox().all();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].from, a_id);
    assert!(
        s.b.inbox().all().is_empty(),
        "B carried it but can't read it"
    );

    // C answers with one tap.
    let reply = Payload::WellnessReply {
        state: CheckInState::Safe,
        text: None,
    };
    s.c.send_direct(a_id, reply, T0 + 6_000).unwrap();
    meet(&s.net, &mut s.c, &mut s.b, T0 + 7_000);
    meet(&s.net, &mut s.b, &mut s.a, T0 + 8_000);
    assert_eq!(s.a.inbox().conversation(a_id, c_id).len(), 2);
}

#[test]
fn neighbors_never_see_family_check_ins() {
    let mut s = street();
    let a_id = s.a.me();
    s.a.record(check_in(a_id, CheckInState::OnMyWay, T0 + 100), T0 + 100)
        .unwrap();
    meet(&s.net, &mut s.a, &mut s.b, T0 + 1_000);

    // B is in A's neighborhood but not A's household: B only carries it.
    assert!(s.b.state().check_in(&a_id).is_none());
    assert!(s.b.carrying_for_others() >= 1);
}

#[test]
fn removal_rotates_the_key_so_the_removed_member_cannot_read_new_posts() {
    let mut s = street();
    let (b_id, c_id) = (s.b.me(), s.c.me());
    meet(&s.net, &mut s.b, &mut s.c, T0 + 1_000);
    meet(&s.net, &mut s.a, &mut s.b, T0 + 2_000);

    // Only stewards can remove; B is not one.
    assert!(matches!(
        s.b.remove_member(s.hood, c_id, RemovalReason::Spam, false, T0 + 3_000),
        Err(SyncError::NotPermitted(_))
    ));

    // The founder removes B without cascade: C stays.
    let epoch =
        s.a.remove_member(s.hood, b_id, RemovalReason::Abuse, false, T0 + 3_000)
            .unwrap();
    assert_eq!(epoch, 1);
    meet(&s.net, &mut s.a, &mut s.c, T0 + 4_000);
    let c_view = s.c.neighborhood(&s.hood).unwrap();
    assert!(!c_view.is_member(&b_id) && c_view.is_member(&c_id));
    assert_eq!(c_view.current_key().unwrap().0, 1, "C received the new key");

    // New posts use the new key: B can carry them but not read them.
    let offer = Payload::Offer {
        text: "Water at the church".into(),
    };
    s.a.post(s.hood, offer, T0 + 5_000).unwrap();
    meet(&s.net, &mut s.a, &mut s.b, T0 + 6_000);
    assert!(s.b.feed(&s.hood).iter().all(|p| p.created_at != T0 + 5_000));
    meet(&s.net, &mut s.a, &mut s.c, T0 + 7_000);
    assert!(s.c.feed(&s.hood).iter().any(|p| p.created_at == T0 + 5_000));
}

#[test]
fn cascade_removal_takes_the_invite_subtree() {
    let mut s = street();
    let (b_id, c_id) = (s.b.me(), s.c.me());
    meet(&s.net, &mut s.b, &mut s.c, T0 + 1_000);
    meet(&s.net, &mut s.a, &mut s.b, T0 + 2_000);
    s.a.remove_member(s.hood, b_id, RemovalReason::Spam, true, T0 + 3_000)
        .unwrap();
    let view = s.a.neighborhood(&s.hood).unwrap();
    assert!(!view.is_member(&b_id) && !view.is_member(&c_id));
    assert_eq!(view.member_count(), 1);
}

#[test]
fn stewards_appointed_by_the_founder_can_remove() {
    let mut s = street();
    let (b_id, c_id) = (s.b.me(), s.c.me());
    meet(&s.net, &mut s.b, &mut s.c, T0 + 1_000);
    meet(&s.net, &mut s.a, &mut s.b, T0 + 2_000);

    assert!(
        s.b.add_steward(s.hood, b_id, T0 + 2_500).is_err(),
        "only the founder appoints"
    );
    s.a.add_steward(s.hood, b_id, T0 + 3_000).unwrap();
    meet(&s.net, &mut s.a, &mut s.b, T0 + 4_000);
    assert!(s.b.neighborhood(&s.hood).unwrap().is_steward(&b_id));
    s.b.remove_member(s.hood, c_id, RemovalReason::LeftArea, false, T0 + 5_000)
        .unwrap();
    assert!(!s.b.neighborhood(&s.hood).unwrap().is_member(&c_id));
}

#[test]
fn rate_limits_and_blocking() {
    let mut s = street();
    for i in 0..20 {
        s.a.post(
            s.hood,
            Payload::Need {
                text: format!("need {i}"),
            },
            T0 + 100 + i,
        )
        .unwrap();
    }
    let extra = Payload::Need {
        text: "one too many".into(),
    };
    assert!(matches!(
        s.a.post(s.hood, extra, T0 + 200),
        Err(SyncError::LimitReached(_))
    ));

    meet(&s.net, &mut s.a, &mut s.b, T0 + 1_000);
    assert_eq!(s.b.feed(&s.hood).len(), 20);
    s.b.block(s.a.me());
    assert!(s.b.feed(&s.hood).is_empty());
}

#[test]
fn people_can_join_at_most_three_neighborhoods() {
    let net = MockNetwork::new();
    let mut a = Family::new("Lee", &["Ana"]).phone(&net, 0);
    for i in 0..3 {
        a.create_neighborhood(&format!("Hood {i}"), "", T0).unwrap();
    }
    assert!(matches!(
        a.create_neighborhood("Hood 4", "", T0),
        Err(SyncError::LimitReached(_))
    ));
}

#[test]
fn invites_are_limited_per_day() {
    let net = MockNetwork::new();
    let mut a = Family::new("Lee", &["Ana"]).phone(&net, 0);
    let hood = a.create_neighborhood("Elm & Oak", "", T0).unwrap();
    for i in 0..10 {
        let name = format!("Guest {i}");
        let guest = Family::new("Guest", &[name.as_str()]).phone(&net, 0);
        // Counted as soon as A signs it, even before the join comes back.
        a.invite(hood, &guest.invite_request(), T0 + i).unwrap();
    }
    let late = Family::new("Late", &["Late"]).phone(&net, 0);
    let result = a.invite(hood, &late.invite_request(), T0 + 500);
    assert!(matches!(result, Err(SyncError::LimitReached(_))));
}
