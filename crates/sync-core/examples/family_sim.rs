//! Store-and-forward walkthrough on the mock transport.
//!
//!     cargo run -p beacon-sync-core --example family_sim
//!
//! Loads the Lee household from `fixtures/sample-household.json`, gives each
//! member with a phone their own engine and keys, adds a neighbor from another
//! household, and walks everyone in and out of range. No radios, no network:
//! everything happens in one process.

use std::collections::HashMap;

use beacon_sync_core::crypto::SymmetricKey;
use beacon_sync_core::protocol::{
    AlertCategory, Envelope, Household, HouseholdId, Payload, PersonId, PersonRole, UnixMillis,
};
use beacon_sync_core::store::MemoryStore;
use beacon_sync_core::transport::{MockNetwork, MockTransport, Transport};
use beacon_sync_core::{LocalIdentity, SyncEngine, SyncEngineConfig};
use serde::Deserialize;

type Phone = SyncEngine<MemoryStore, MockTransport>;

#[derive(Deserialize)]
struct Fixture {
    generated_at: UnixMillis,
    household: Household,
    /// device id -> person id
    devices: HashMap<String, PersonId>,
    envelopes: Vec<Envelope>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/sample-household.json"
    );
    let fixture: Fixture = serde_json::from_str(&std::fs::read_to_string(path)?)?;
    let mut household = fixture.household;
    let names: HashMap<PersonId, String> = household
        .members
        .iter()
        .map(|p| (p.id, p.display_name.clone()))
        .collect();
    let name = |id: &PersonId| {
        names
            .get(id)
            .cloned()
            .unwrap_or_else(|| "a neighbor".into())
    };

    // In-person pairing: every member with a phone gets keys, and the
    // household shares one key.
    let mut identities: HashMap<PersonId, LocalIdentity> = HashMap::new();
    for person in fixture.devices.values() {
        let identity = LocalIdentity {
            person_id: *person,
            ..LocalIdentity::generate(name(person))
        };
        if let Some(member) = household.members.iter_mut().find(|m| m.id == *person) {
            *member = identity.as_person(member.role);
        }
        identities.insert(*person, identity);
    }
    let household_key = SymmetricKey::generate();

    let net = MockNetwork::new();
    let mut phones: HashMap<PersonId, Phone> = HashMap::new();
    for (device, person) in &fixture.devices {
        let Some(identity) = identities.remove(person) else {
            continue;
        };
        let device = beacon_sync_core::protocol::DeviceId::parse(device)?;
        let config =
            SyncEngineConfig::new(device, identity, household.clone(), household_key.clone());
        let phone = SyncEngine::new(
            config,
            MemoryStore::new(),
            net.join(name(person).as_str()),
            0,
        )?;
        phones.insert(*person, phone);
    }

    // A neighbor from a different household, with their own phone.
    let dana = LocalIdentity::generate("Dana");
    let dana_id = dana.person_id;
    let dana_home = Household {
        id: HouseholdId::new(),
        name: "The Okafor household".into(),
        members: vec![dana.as_person(PersonRole::Adult)],
    };
    let config = SyncEngineConfig::new(
        Default::default(),
        dana,
        dana_home,
        SymmetricKey::generate(),
    );
    let mut dana_phone = SyncEngine::new(config, MemoryStore::new(), net.join("Dana"), 0)?;

    println!("== Everyone is apart; each Lee phone records its own updates");
    for env in &fixture.envelopes {
        if let Some(phone) = phones.get_mut(&env.author) {
            phone.record(env.payload.clone(), env.created_at)?;
        }
    }
    print_views(&phones, &name);

    let ids: Vec<PersonId> = household.members.iter().map(|p| p.id).collect();
    let (sam, jordan, maya) = (ids[0], ids[1], ids[2]);
    let mut now = fixture.generated_at;

    println!("\n== Jordan walks past Dana's house. Dana is not family.");
    meet(&net, phones.get_mut(&jordan), Some(&mut dana_phone), now);
    println!(
        "   Dana's phone is carrying {} sealed envelopes it cannot read",
        dana_phone.carrying_for_others()
    );

    now += 10 * 60_000;
    println!("\n== Dana walks on and passes Maya at the school gate");
    meet(&net, Some(&mut dana_phone), phones.get_mut(&maya), now);
    print_views(&phones, &name);
    println!("   Maya now has Jordan's check-in, carried by a neighbor who couldn't read it.");

    now += 10 * 60_000;
    println!("\n== Sam starts an 'Elm & Oak' neighborhood and invites Dana in person");
    let sam_phone = phones.get_mut(&sam).ok_or("Sam has no phone")?;
    let hood = sam_phone.create_neighborhood("Elm & Oak", "Elm and Oak streets", now)?;
    let package = sam_phone.invite(hood, &dana_phone.invite_request(), now)?;
    dana_phone.accept_invite(package, now)?;
    dana_phone.post(
        hood,
        Payload::Alert {
            category: AlertCategory::Power,
            text: "Transformer down at Elm & 5th".into(),
            area: Some("Elm St, 400 block".into()),
        },
        now,
    )?;
    meet(
        &net,
        Some(&mut dana_phone),
        phones.get_mut(&sam),
        now + 60_000,
    );

    if let Some(sam_phone) = phones.get(&sam) {
        let members = sam_phone
            .neighborhood(&hood)
            .map_or(0, |s| s.member_count());
        println!("   Sam's neighborhood has {members} members");
        for post in sam_phone.feed(&hood) {
            println!(
                "   Sam sees a post from {}: {:?}",
                if post.author == dana_id { "Dana" } else { "?" },
                post.kind
            );
        }
        println!(
            "   Dana still can't read the Lee family's check-ins: {}",
            dana_phone.state().check_ins().is_empty()
        );
    }
    println!("\nFrames sent over the mock network: {}", net.frames_sent());
    Ok(())
}

/// Bring two phones into range, run rounds until quiet, then separate them.
fn meet(net: &MockNetwork, a: Option<&mut Phone>, b: Option<&mut Phone>, now: UnixMillis) {
    let (Some(a), Some(b)) = (a, b) else { return };
    let (addr_a, addr_b) = (
        a.transport_mut().local_address(),
        b.transport_mut().local_address(),
    );
    net.set_in_range(&addr_a, &addr_b, true);
    for _ in 0..10 {
        let before = net.frames_sent();
        for (phone, label) in [(&mut *a, &addr_a), (&mut *b, &addr_b)] {
            let r = phone.sync_round(now);
            if r.exchanged_anything() {
                println!(
                    "   {label}: sent {}, received {}, carrying {} for others",
                    r.envelopes_sent, r.envelopes_received, r.envelopes_relayed
                );
            }
        }
        if net.frames_sent() == before {
            break;
        }
    }
    net.set_in_range(&addr_a, &addr_b, false);
}

fn print_views(phones: &HashMap<PersonId, Phone>, name: &dyn Fn(&PersonId) -> String) {
    let mut owners: Vec<_> = phones.keys().copied().collect();
    owners.sort_by_key(name);
    for owner in owners {
        let state = phones[&owner].state();
        let check_ins: Vec<String> = state
            .check_ins()
            .iter()
            .map(|c| format!("{}={:?}", name(&c.person_id), c.state))
            .collect();
        println!(
            "   {:<7} sees {} check-ins [{}], {} meeting points, {} notes",
            name(&owner),
            check_ins.len(),
            check_ins.join(", "),
            state.rendezvous_points().len(),
            state.notes().len()
        );
    }
}
