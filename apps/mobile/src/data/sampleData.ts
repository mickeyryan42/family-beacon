/**
 * Loads the shared development dataset from /fixtures.
 *
 * The fixture is frozen at a fixed `generated_at` time so Rust tests are
 * deterministic. Here we shift every timestamp forward so the newest update
 * looks a few minutes old, otherwise every check-in would already be expired.
 */
import fixture from '../../../../fixtures/sample-household.json';
import type {Envelope, Household, Payload} from '../sync/protocol';
import {DEFAULT_LOCAL_STATE, type LocalSnapshot} from './repository';

const NEWEST_AGE_MS = 4 * 60_000;

interface Fixture {
  generated_at: number;
  local_person_id: string;
  local_device_id: string;
  household: Household;
  envelopes: Envelope[];
}

function shiftPayload(payload: Payload, offset: number): Payload {
  if (payload.type === 'check_in') {
    return {...payload, data: {...payload.data, recorded_at: payload.data.recorded_at + offset}};
  }
  return payload;
}

export function sampleSnapshot(now: number = Date.now()): LocalSnapshot {
  const data = fixture as unknown as Fixture;
  const newest = Math.max(...data.envelopes.map(e => e.created_at));
  const offset = now - NEWEST_AGE_MS - newest;

  return {
    household: data.household,
    device: {deviceId: data.local_device_id, personId: data.local_person_id},
    envelopes: data.envelopes.map(e => ({
      ...e,
      created_at: e.created_at + offset,
      expires_at: e.expires_at + offset,
      payload: shiftPayload(e.payload, offset),
    })),
    carried: [],
    local: DEFAULT_LOCAL_STATE,
  };
}
