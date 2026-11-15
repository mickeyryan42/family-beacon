use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;

const T0: UnixMillis = 1_800_000_000_000;

/// A test phone: its own database file and a clock the test moves.
struct Phone {
    core: BeaconCore,
    db: PathBuf,
    clock: Arc<AtomicU64>,
}

impl Phone {
    fn create(household: &str, name: &str) -> Self {
        let db = temp_db();
        let clock = Arc::new(AtomicU64::new(T0));
        let core =
            BeaconCore::create_with_clock(db.to_str().unwrap(), household, name, clock_fn(&clock))
                .unwrap();
        Self { core, db, clock }
    }

    /// Simulate the app being killed and relaunched with the saved keys.
    fn restart(mut self) -> Self {
        let secrets = self.core.secrets().unwrap();
        // Assigning drops the old core (and its connection).
        self.core =
            BeaconCore::open_with_clock(self.db.to_str().unwrap(), &secrets, clock_fn(&self.clock))
                .unwrap();
        self
    }

    fn advance(&self, ms: u64) {
        self.clock.fetch_add(ms, Ordering::SeqCst);
    }

    fn snapshot(&self) -> Value {
        serde_json::from_str(&self.core.snapshot_json().unwrap()).unwrap()
    }

    fn me(&self) -> String {
        self.snapshot()["personId"].as_str().unwrap().to_owned()
    }
}

impl Drop for Phone {
    fn drop(&mut self) {
        let _ = erase_stored_circle(self.db.to_string_lossy().into_owned());
    }
}

fn temp_db() -> PathBuf {
    std::env::temp_dir().join(format!("beacon-ffi-{}.db", DeviceId::new()))
}

fn clock_fn(clock: &Arc<AtomicU64>) -> Clock {
    let clock = Arc::clone(clock);
    Box::new(move || clock.load(Ordering::SeqCst))
}

/// Two phones side by side: run rounds and carry frames between them the
/// way the native radio layer will.
fn meet(a: &Phone, b: &Phone) {
    let (addr_a, addr_b) = (
        a.core.local_address().unwrap(),
        b.core.local_address().unwrap(),
    );
    for (phone, other) in [(a, &addr_b), (b, &addr_a)] {
        phone.core.set_radio_available(true).unwrap();
        phone.core.set_peers(vec![other.clone()]).unwrap();
    }
    for _ in 0..10 {
        let mut moved = 0;
        for (from, to, from_addr) in [(a, b, &addr_a), (b, a, &addr_b)] {
            from.core.sync_round().unwrap();
            for frame in from.core.take_outbound().unwrap() {
                assert_eq!(
                    &frame.to,
                    if std::ptr::eq(to, b) {
                        &addr_b
                    } else {
                        &addr_a
                    }
                );
                assert!(to
                    .core
                    .receive_frame(from_addr.clone(), frame.bytes)
                    .unwrap());
                moved += 1;
            }
        }
        if moved == 0 {
            break;
        }
    }
}

fn check_in(person: &str, state: &str) -> String {
    json!({"type": "check_in", "data": {"person_id": person, "state": state, "recorded_at": T0}})
        .to_string()
}

#[test]
fn creates_a_circle_and_renders_the_snapshot_in_the_app_shape() {
    let mom = Phone::create("The Okafors", "Mom");
    let me = mom.me();
    mom.core.record(check_in(&me, "on_my_way")).unwrap();
    mom.core
        .record(json!({"type": "note", "data": {"text": "Power's out"}}).to_string())
        .unwrap();

    let snap = mom.snapshot();
    assert_eq!(snap["household"]["name"], "The Okafors");
    assert_eq!(snap["household"]["members"][0]["display_name"], "Mom");
    assert_eq!(snap["view"]["checkIns"][&me]["state"], "on_my_way");
    assert_eq!(snap["view"]["notes"][0]["text"], "Power's out");
    assert!(snap["view"]["notes"][0]["envelopeId"].is_string());
    assert_eq!(snap["relayEnabled"], true);
    assert_eq!(snap["pendingOutbound"], 2);
}

#[test]
fn rejects_bad_input_with_typed_errors() {
    let mom = Phone::create("The Okafors", "Mom");
    assert!(matches!(
        mom.core.record("{not json".into()),
        Err(CoreError::Invalid { .. })
    ));
    let too_long = json!({"type": "note", "data": {"text": "x".repeat(5_000)}}).to_string();
    assert!(matches!(
        mom.core.record(too_long),
        Err(CoreError::Invalid { .. })
    ));
    assert!(matches!(
        mom.core.block("not-a-uuid".into()),
        Err(CoreError::Invalid { .. })
    ));
    // A second circle can't silently replace the first.
    let again =
        BeaconCore::create_with_clock(mom.db.to_str().unwrap(), "Other", "X", clock_fn(&mom.clock));
    assert!(matches!(again, Err(CoreError::NotPermitted { .. })));
    assert!(matches!(
        BeaconCore::create_with_clock(temp_db().to_str().unwrap(), " ", "X", Box::new(|| T0)),
        Err(CoreError::Invalid { .. })
    ));
}

#[test]
fn restart_restores_updates_settings_and_neighborhoods() {
    let mom = Phone::create("The Okafors", "Mom");
    let me = mom.me();
    let stranger = PersonId::new().to_string();
    mom.core.record(check_in(&me, "safe")).unwrap();
    let hood = mom
        .core
        .create_neighborhood("Elm & Oak".into(), "Elm St".into())
        .unwrap();
    mom.core
        .post(
            hood.clone(),
            json!({"type": "offer", "data": {"text": "Spare water"}}).to_string(),
        )
        .unwrap();
    mom.core.set_relay_enabled(false).unwrap();
    mom.core.block(stranger.clone()).unwrap();
    let db = mom.db.to_string_lossy().into_owned();
    assert!(has_stored_circle(db.clone()).unwrap());

    let mom = mom.restart();
    let snap = mom.snapshot();
    assert_eq!(snap["personId"], me.as_str());
    assert_eq!(snap["view"]["checkIns"][&me]["state"], "safe");
    assert_eq!(snap["relayEnabled"], false);
    assert_eq!(snap["blocked"], json!([stranger]));
    assert_eq!(snap["neighborhoods"][0]["info"]["id"], hood.as_str());
    assert_eq!(snap["neighborhoods"][0]["isFounder"], true);
    assert_eq!(snap["neighborhoods"][0]["feed"][0]["kind"], "offer");
    assert_eq!(snap["neighborhoods"][0]["feed"][0]["text"], "Spare water");
    // Still able to author under the restored keys.
    mom.core.record(check_in(&me, "at_rendezvous")).unwrap();
    // Erasing removes the circle entirely.
    erase_stored_circle(db.clone()).unwrap();
    assert!(!has_stored_circle(db).unwrap());
}

#[test]
fn keys_are_tied_to_their_install_and_change_detection_works() {
    let mom = Phone::create("The Okafors", "Mom");
    let other = Phone::create("The Webbs", "Marcus");
    let secrets = mom.core.secrets().unwrap();
    assert_eq!(mom.core.take_changed_secrets().unwrap(), None);

    // Joining a neighborhood adds a key to keep.
    mom.core
        .create_neighborhood("Elm & Oak".into(), String::new())
        .unwrap();
    let changed = mom.core.take_changed_secrets().unwrap().expect("new key");
    assert_ne!(changed, secrets);
    assert_eq!(mom.core.take_changed_secrets().unwrap(), None);

    let wrong = BeaconCore::open_with_clock(
        mom.db.to_str().unwrap(),
        &other.core.secrets().unwrap(),
        Box::new(|| T0),
    );
    assert!(matches!(wrong, Err(CoreError::Secrets { .. })));
    let garbage = BeaconCore::open_with_clock(mom.db.to_str().unwrap(), "{}", Box::new(|| T0));
    assert!(matches!(garbage, Err(CoreError::Secrets { .. })));
}

#[test]
fn neighbors_invite_in_person_then_sync_over_bridged_frames() {
    let dana = Phone::create("The Okafors", "Dana");
    let marcus = Phone::create("The Webbs", "Marcus");
    let hood = dana
        .core
        .create_neighborhood("Elm & Oak".into(), "Elm St".into())
        .unwrap();

    // In person: Marcus shows his request, Dana scans it and hands back a package.
    let request = marcus.core.invite_request_json().unwrap();
    let package = dana.core.invite(hood.clone(), request).unwrap();
    assert_eq!(marcus.core.accept_invite(package).unwrap(), hood);
    assert!(marcus.core.take_changed_secrets().unwrap().is_some());

    dana.advance(60_000);
    marcus.advance(60_000);
    dana.core
        .post(
            hood.clone(),
            json!({"type": "alert", "data": {"category": "power", "text": "Out on Elm"}})
                .to_string(),
        )
        .unwrap();

    meet(&dana, &marcus);

    let snap = marcus.snapshot();
    let feed = &snap["neighborhoods"][0]["feed"];
    assert_eq!(feed[0]["kind"], "alert", "{snap:#}");
    assert_eq!(feed[0]["category"], "power");
    assert_eq!(feed[0]["text"], "Out on Elm");
    let names: Vec<_> = snap["neighborhoods"][0]["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["display_name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["Dana", "Marcus"]);
    // Family check-ins never cross into another household.
    assert!(snap["view"]["checkIns"].as_object().unwrap().is_empty());

    // Private "Are you OK?", answered after the next meeting.
    let marcus_id = marcus.me();
    dana.core
        .send_direct(
            marcus_id,
            json!({"type": "wellness_request", "data": {"text": "Are you OK?"}}).to_string(),
        )
        .unwrap();
    dana.advance(10 * 60_000);
    marcus.advance(10 * 60_000);
    meet(&dana, &marcus);
    assert_eq!(marcus.snapshot()["inbox"][0]["kind"], "wellness_request");
}

#[test]
fn the_radio_seam_respects_availability_and_frame_limits() {
    let a = Phone::create("A", "A");
    let report = a.core.sync_round().unwrap();
    assert!(
        report.skipped.is_some(),
        "radio is off until native says so"
    );
    a.core.set_radio_available(true).unwrap();
    a.core.set_peers(vec!["peer-1".into()]).unwrap();
    let report = a.core.sync_round().unwrap();
    assert_eq!(report.peers_in_range, 1);
    assert_eq!(report.sessions_started, 1);
    assert_eq!(report.frames_to_send, 1);
    let frames = a.core.take_outbound().unwrap();
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].to, "peer-1");
    assert!(a.core.take_outbound().unwrap().is_empty());

    // Garbage from a peer is counted as an error, never a crash.
    assert!(a
        .core
        .receive_frame("peer-1".into(), vec![0xff; 32])
        .unwrap());
    assert!(a.core.sync_round().unwrap().errors >= 1);
    assert!(a.core.next_sync_in_ms().unwrap() > 0 || a.core.should_sync().unwrap());
}
