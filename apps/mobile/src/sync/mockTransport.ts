/**
 * Mock offline transport for development.
 *
 * Simulates the other phones around you wandering into range every so often:
 *
 * - **Family phones** (household members other than you): head to the meeting
 *   point, check in, leave a note.
 * - **Neighbor phones** (members of a neighborhood you've joined): post
 *   alerts, offers and needs, share "all safe", check on you with
 *   "Are you OK?", and answer your messages.
 * - **A passer-by** from another household: hands over sealed traffic your
 *   phone can't read and can only carry for others.
 *
 * Nothing touches a radio or the network, so the whole sync flow can be
 * exercised on a simulator or in Jest.
 */
import {logger} from '../lib/logger';
import {
  createEnvelope,
  devUuid,
  type Audience,
  type Envelope,
  type Household,
  type Payload,
  type RendezvousPoint,
  type Uuid,
} from './protocol';
import type {
  Encounter,
  NearbyPeer,
  SyncTransport,
  TransportListener,
  TransportStatus,
} from './transport';

const log = logger('mock-transport');

export interface MockNeighborhood {
  id: Uuid;
  /** Members other than you and your household, who have phones. */
  members: {id: Uuid; display_name: string}[];
}

export interface MockContext {
  household: Household;
  localPersonId: Uuid;
  rendezvous: RendezvousPoint[];
  neighborhoods: MockNeighborhood[];
}

export interface MockTransportOptions {
  /** How often a simulated phone comes into range. 0 disables the timer. */
  exchangeIntervalMs: number;
  /** How long a simulated phone stays in range after an encounter. */
  inRangeMs?: number;
  /** Clock override for tests. */
  now?: () => number;
}

type Step = (ctx: MockContext, now: number) => {audience: Audience; payload: Payload} | null;

const household = (ctx: MockContext): Audience => ({
  scope: 'household',
  household_id: ctx.household.id,
});

function familyScript(personId: Uuid, isChild: boolean): Step[] {
  const checkIn =
    (state: 'safe' | 'on_my_way' | 'at_rendezvous', note: string, atPoint = false): Step =>
    (ctx, now) => ({
      audience: household(ctx),
      payload: {
        type: 'check_in',
        data: {
          person_id: personId,
          state,
          note,
          rendezvous_id: atPoint ? ctx.rendezvous[0]?.id : undefined,
          recorded_at: now,
        },
      },
    });
  if (isChild) {
    return [
      checkIn('safe', 'Still at school. Teachers are with us.'),
      checkIn('on_my_way', 'Walking with the group', true),
      checkIn('safe', 'Home safe.'),
    ];
  }
  return [
    (ctx, now) =>
      checkIn('on_my_way', `Heading to ${ctx.rendezvous[0]?.name ?? 'home'}`, true)(ctx, now),
    checkIn('at_rendezvous', 'Made it. Waiting here.', true),
    ctx => ({
      audience: household(ctx),
      payload: {type: 'note', data: {text: 'Phones are charged.'}},
    }),
    checkIn('safe', 'All good. Staying put for now.'),
  ];
}

function neighborScript(neighborhoodId: Uuid, name: string): Step[] {
  const hood: Audience = {scope: 'neighborhood', neighborhood_id: neighborhoodId};
  const post =
    (payload: Payload): Step =>
    () => ({audience: hood, payload});
  return [
    post({
      type: 'alert',
      data: {category: 'power', text: 'Transformer down at Elm & 5th', area: 'Elm St, 400 block'},
    }),
    post({
      type: 'offer',
      data: {text: `Generator at ${name.split(' ')[0]}'s. Come charge phones until 8pm.`},
    }),
    ctx => ({
      audience: {scope: 'direct', recipient: ctx.localPersonId},
      payload: {type: 'wellness_request', data: {text: 'Are you OK? Saw your lights were out.'}},
    }),
    post({type: 'need', data: {text: 'Need ice for insulin. Anyone have a cooler?'}}),
    post({
      type: 'household_status',
      data: {household_name: `The ${name.split(' ').pop()} household`, all_safe: true},
    }),
  ];
}

class SimulatedPhone {
  readonly deviceId = devUuid();
  readonly holds = new Map<Uuid, Envelope>();
  private step = 0;
  private lamport = 0;
  /** Replies owed to messages this phone received. */
  private pending: Step[] = [];

  constructor(
    readonly address: string,
    readonly label: string,
    readonly personId: Uuid,
    private readonly script: Step[],
  ) {}

  receive(env: Envelope) {
    if (this.holds.has(env.id)) {
      return;
    }
    this.holds.set(env.id, env);
    if (env.audience.scope !== 'direct' || env.audience.recipient !== this.personId) {
      return;
    }
    const to = env.author;
    if (env.payload.type === 'wellness_request') {
      this.pending.push(() => ({
        audience: {scope: 'direct', recipient: to},
        payload: {
          type: 'wellness_reply',
          data: {state: 'safe', text: "We're fine, thanks for checking."},
        },
      }));
    } else if (env.payload.type === 'message') {
      this.pending.push(() => ({
        audience: {scope: 'direct', recipient: to},
        payload: {type: 'message', data: {text: 'Got your message. Thanks!'}},
      }));
    }
  }

  /** Produce this person's next update, if they have one. */
  act(ctx: MockContext, now: number): Envelope | null {
    const next = this.pending.shift() ?? this.script[this.step++];
    const made = next?.(ctx, now);
    if (!made) {
      return null;
    }
    for (const env of this.holds.values()) {
      this.lamport = Math.max(this.lamport, env.lamport);
    }
    this.lamport += 1;
    const env = createEnvelope({
      ...made,
      author: this.personId,
      origin_device: this.deviceId,
      lamport: this.lamport,
      now,
    });
    this.holds.set(env.id, env);
    return env;
  }
}

/** Someone from another household walking by: carries sealed traffic for others. */
function passerByScript(): Step[] {
  const otherHousehold = devUuid();
  const someone = devUuid();
  const sealed: Step = (_ctx, now) => ({
    audience: {scope: 'household', household_id: otherHousehold},
    payload: {
      type: 'check_in',
      data: {person_id: someone, state: 'safe', recorded_at: now},
    },
  });
  return [sealed, sealed, sealed];
}

export class MockTransport implements SyncTransport {
  readonly kind = 'mock' as const;
  readonly label = 'Mock transport (development)';

  private listener: TransportListener | null = null;
  private timer: ReturnType<typeof setInterval> | null = null;
  private leaveTimer: ReturnType<typeof setTimeout> | null = null;
  private phones = new Map<string, SimulatedPhone>();
  private turn = 0;
  private readonly now: () => number;

  constructor(
    private readonly getContext: () => MockContext | null,
    private readonly options: MockTransportOptions,
  ) {
    this.now = options.now ?? Date.now;
  }

  status(): TransportStatus {
    return this.listener ? {state: 'running'} : {state: 'stopped'};
  }

  start(listener: TransportListener): void {
    this.stop();
    this.listener = listener;
    if (this.options.exchangeIntervalMs > 0) {
      this.timer = setInterval(() => this.encounterNext(), this.options.exchangeIntervalMs);
    }
    log.info('started', {intervalMs: this.options.exchangeIntervalMs});
  }

  stop(): void {
    if (this.timer) {
      clearInterval(this.timer);
    }
    if (this.leaveTimer) {
      clearTimeout(this.leaveTimer);
    }
    this.timer = null;
    this.leaveTimer = null;
    if (this.listener) {
      this.listener.onPeersChanged([]);
      log.info('stopped');
    }
    this.listener = null;
  }

  /**
   * Bring the next simulated phone into range right now. Exposed for the
   * "Simulate a phone nearby" button and for tests. `kind` limits which sort
   * of phone comes by.
   */
  encounterNext(kind?: 'family' | 'neighbor' | 'passer-by'): boolean {
    const ctx = this.getContext();
    if (!ctx || !this.listener) {
      return false;
    }
    const candidates = this.candidates(ctx).filter(
      p => !kind || p.address.startsWith(`mock:${kind}`),
    );
    if (candidates.length === 0) {
      log.debug('no simulated phones to bring into range');
      return false;
    }
    const phone = candidates[this.turn % candidates.length];
    this.turn += 1;

    const created = phone.act(ctx, this.now());
    if (created) {
      log.debug(`${phone.label} made an update while apart`, {kind: created.payload.type});
    }

    const peer: NearbyPeer = {address: phone.address, personId: phone.personId, label: phone.label};
    const encounter: Encounter = {
      peer,
      peerHas: new Set(phone.holds.keys()),
      exchange: async (outbound, localHave) => {
        outbound.forEach(env => phone.receive(env));
        return [...phone.holds.values()].filter(env => !localHave.has(env.id));
      },
    };

    this.listener.onPeersChanged([peer]);
    this.listener.onEncounter(encounter);

    if (this.leaveTimer) {
      clearTimeout(this.leaveTimer);
    }
    this.leaveTimer = setTimeout(() => {
      this.listener?.onPeersChanged([]);
    }, this.options.inRangeMs ?? 8_000);
    return true;
  }

  private phone(address: string, make: () => SimulatedPhone): SimulatedPhone {
    let phone = this.phones.get(address);
    if (!phone) {
      phone = make();
      this.phones.set(address, phone);
    }
    return phone;
  }

  private candidates(ctx: MockContext): SimulatedPhone[] {
    const out: SimulatedPhone[] = [];
    // Family: everyone but you; caregivers are assumed not to carry the app.
    for (const m of ctx.household.members) {
      if (m.id === ctx.localPersonId || m.role === 'caregiver') {
        continue;
      }
      out.push(
        this.phone(
          `mock:family:${m.id}`,
          () =>
            new SimulatedPhone(
              `mock:family:${m.id}`,
              `${m.display_name}'s phone`,
              m.id,
              familyScript(m.id, m.role === 'child'),
            ),
        ),
      );
    }
    const familyIds = new Set(ctx.household.members.map(m => m.id));
    for (const hood of ctx.neighborhoods) {
      for (const m of hood.members) {
        if (m.id === ctx.localPersonId || familyIds.has(m.id)) {
          continue;
        }
        const address = `mock:neighbor:${hood.id}:${m.id}`;
        out.push(
          this.phone(
            address,
            () =>
              new SimulatedPhone(
                address,
                `${m.display_name}'s phone`,
                m.id,
                neighborScript(hood.id, m.display_name),
              ),
          ),
        );
      }
    }
    out.push(
      this.phone(
        'mock:passer-by',
        () =>
          new SimulatedPhone('mock:passer-by', "A passer-by's phone", devUuid(), passerByScript()),
      ),
    );
    return out;
  }
}
