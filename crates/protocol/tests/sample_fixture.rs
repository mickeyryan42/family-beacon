//! Contract test: the shared development fixture must stay valid wire format.
//!
//! The mobile app's mock transport feeds these exact envelopes into the UI, so
//! if a protocol change breaks this test, the fixture (and the TypeScript
//! mirror in `apps/mobile/src/sync/protocol.ts`) must be updated with it.

use beacon_protocol::{
    decode_envelope, encode_envelope, validate_envelope, Envelope, Household, Membership,
    NeighborhoodId, PersonId, Scope, UnixMillis, ValidationLimits,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    generated_at: UnixMillis,
    local_person_id: PersonId,
    household: Household,
    envelopes: Vec<Envelope>,
}

impl Membership for Fixture {
    fn local_person(&self) -> PersonId {
        self.local_person_id
    }
    fn household(&self) -> &Household {
        &self.household
    }
    fn in_neighborhood(&self, _: &NeighborhoodId) -> bool {
        false
    }
    fn is_neighborhood_member(&self, _: &NeighborhoodId, _: &PersonId) -> bool {
        false
    }
    fn knows(&self, p: &PersonId) -> bool {
        self.household.is_member(p)
    }
}

fn load() -> Fixture {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/sample-household.json"
    );
    let raw = std::fs::read_to_string(path).expect("fixtures/sample-household.json is readable");
    serde_json::from_str(&raw).expect("fixture matches the protocol types")
}

#[test]
fn every_fixture_envelope_is_valid_at_generation_time() {
    let fixture = load();
    assert!(!fixture.envelopes.is_empty());
    for env in &fixture.envelopes {
        let result = validate_envelope(
            env,
            &fixture,
            fixture.generated_at,
            &ValidationLimits::default(),
        );
        assert_eq!(result, Ok(()), "envelope {} is invalid", env.id);
    }
}

#[test]
fn fixture_envelopes_round_trip_through_the_codec() {
    for env in load().envelopes {
        let decoded = decode_envelope(&encode_envelope(&env).unwrap()).unwrap();
        assert_eq!(decoded, env);
    }
}

#[test]
fn fixture_covers_every_household_payload_the_ui_renders() {
    let fixture = load();
    let kinds: Vec<_> = fixture.envelopes.iter().map(|e| e.payload.kind()).collect();
    for wanted in ["check_in", "rendezvous_upsert", "note"] {
        assert!(kinds.contains(&wanted), "fixture has no {wanted} envelope");
    }
    assert!(fixture
        .envelopes
        .iter()
        .all(|e| e.scope() == Scope::Household));
}
