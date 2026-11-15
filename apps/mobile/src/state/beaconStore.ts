/**
 * The app's single source of state.
 *
 * Owns the repository, the active transport and the derived views. Screens
 * read immutable snapshots through `useBeacon()` and call the action methods
 * below; they never talk to storage or transports directly.
 *
 * Local actions follow the same path as remote updates: build an envelope,
 * validate it, store it, re-derive the views. That symmetry is what makes the
 * app behave identically with or without anyone nearby.
 *
 * Scopes (docs/neighborhood-network.md):
 * - household: family only
 * - neighborhood: members of a neighborhood this phone has joined
 * - direct: one person
 * Anything this phone can't read is carried for others when relaying is on.
 * The Rust core enforces that with encryption; this mirror enforces the same
 * rule by audience (see sync/protocol.ts).
 */
import {config} from '../config';
import {describeError, logger} from '../lib/logger';
import {
  type DeviceProfile,
  type IssuedInvite,
  type LocalState,
  type Repository,
} from '../data/repository';
import {StorageRepository} from '../data/storageRepository';
import {sampleSnapshot} from '../data/sampleData';
import {buildView, type HouseholdView} from '../sync/householdView';
import {MockTransport, type MockNeighborhood} from '../sync/mockTransport';
import {NativeTransportStub} from '../sync/nativeTransport';
import {
  buildFeed,
  buildInbox,
  buildRoster,
  DIRECT_PER_DAY,
  INVITES_PER_DAY,
  invitedSubtree,
  inviteChain,
  MAX_NEIGHBORHOODS,
  POSTS_PER_DAY,
  SOFT_MEMBER_CAP,
  usedToday,
  type DirectMessage,
  type Post,
  type Roster,
} from '../sync/neighborhoodView';
import {
  createEnvelope,
  DAY_MS,
  devUuid,
  isExpired,
  isMembershipEvent,
  validateEnvelope,
  type AlertCategory,
  type Audience,
  type CheckInState,
  type Envelope,
  type Household,
  type MembershipCert,
  type Neighborhood,
  type Payload,
  type PersonRole,
  type ReaderContext,
  type RemovalReason,
  type RendezvousPoint,
  type Uuid,
} from '../sync/protocol';
import type {
  Encounter,
  NearbyPeer,
  SyncTransport,
  TransportKind,
  TransportStatus,
} from '../sync/transport';

const log = logger('store');
const MAX_ACTIVITY = 20;

export type ActionResult = {ok: true} | {ok: false; error: string};

export interface ExchangeRecord {
  at: number;
  peerLabel: string;
  sent: number;
  received: number;
  /** Sealed envelopes picked up to carry for others. */
  carried: number;
  rejected: number;
}

export interface NeighborhoodSummary {
  info: Neighborhood;
  /** Current members, sorted by name. */
  members: MembershipCert[];
  stewards: Uuid[];
  isFounder: boolean;
  isSteward: boolean;
  /** This phone's person was removed; new posts are no longer readable. */
  removedMe: boolean;
  feed: Post[];
  invitesLeftToday: number;
  postsLeftToday: number;
  overSoftCap: boolean;
}

export interface BeaconSnapshot {
  phase: 'loading' | 'onboarding' | 'ready';
  household: Household | null;
  device: DeviceProfile | null;
  view: HouseholdView;
  envelopeCount: number;
  neighborhoods: NeighborhoodSummary[];
  /** Direct messages to and from this person, oldest first. */
  inbox: DirectMessage[];
  directLeftToday: number;
  /** Display names for everyone this phone knows. */
  people: Record<Uuid, string>;
  blocked: Uuid[];
  relayEnabled: boolean;
  carryingForOthers: number;
  transport: {kind: TransportKind; label: string; status: TransportStatus};
  nearby: NearbyPeer[];
  activity: ExchangeRecord[];
  /** Developer toggle: render the app as if there were no signal at all. */
  simulateNoSignal: boolean;
  error: string | null;
}

const EMPTY_VIEW: HouseholdView = {checkIns: {}, rendezvous: [], notes: []};

export class BeaconStore {
  private snapshot: BeaconSnapshot;
  private listeners = new Set<() => void>();
  /** Envelopes this phone can read, including its own. */
  private envelopes: Envelope[] = [];
  /** Sealed traffic carried for others (unreadable here). */
  private carrying = new Map<Uuid, Envelope>();
  private joined = new Map<Uuid, Neighborhood>();
  private rosters = new Map<Uuid, Roster>();
  /** Invites signed here, counted toward the daily limit immediately. */
  private issuedInvites: IssuedInvite[] = [];
  private blocked = new Set<Uuid>();
  private relayEnabled = true;
  private transport: SyncTransport;
  /** What each peer address is known to hold, so we don't resend it. */
  private peerHoldings = new Map<string, Set<Uuid>>();
  /** Lamport floor; see LocalState.lamport. */
  private lamportFloor = 0;
  private ticker: ReturnType<typeof setInterval> | null = null;

  constructor(
    private readonly repo: Repository = new StorageRepository(),
    private readonly now: () => number = Date.now,
    transportKind: TransportKind = config.defaultTransport,
  ) {
    this.transport = this.makeTransport(transportKind);
    this.snapshot = {
      phase: 'loading',
      household: null,
      device: null,
      view: EMPTY_VIEW,
      envelopeCount: 0,
      neighborhoods: [],
      inbox: [],
      directLeftToday: DIRECT_PER_DAY,
      people: {},
      blocked: [],
      relayEnabled: true,
      carryingForOthers: 0,
      transport: this.describeTransport(),
      nearby: [],
      activity: [],
      simulateNoSignal: false,
      error: null,
    };
  }

  // --- subscription (for useSyncExternalStore) ---

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  getSnapshot = (): BeaconSnapshot => this.snapshot;

  private update(patch: Partial<BeaconSnapshot>) {
    this.snapshot = {...this.snapshot, ...patch};
    this.listeners.forEach(l => l());
  }

  // --- lifecycle ---

  async init(): Promise<void> {
    try {
      const saved = await this.repo.load();
      if (!saved) {
        this.update({phase: 'onboarding'});
        return;
      }
      this.envelopes = saved.envelopes;
      this.restoreLocalState(saved.local, saved.carried);
      await this.pruneExpired();
      this.update({
        phase: 'ready',
        household: saved.household,
        device: saved.device,
        relayEnabled: this.relayEnabled,
      });
      this.refreshView();
      this.startTransport();
      this.startTicker();
    } catch (e) {
      this.fail('Could not open local data', e);
    }
  }

  /** Stop radio work and timers. Subscribers are left alone; React removes its own. */
  dispose(): void {
    this.transport.stop();
    this.stopTicker();
  }

  /**
   * Drop expired updates and re-derive every view against the current time.
   * Runs on a timer and whenever the app returns to the foreground.
   */
  async refresh(): Promise<void> {
    if (this.snapshot.phase !== 'ready') {
      return;
    }
    try {
      await this.pruneExpired();
    } catch (e) {
      log.warn('could not prune expired updates', describeError(e));
    }
    this.refreshView();
  }

  private startTicker() {
    this.stopTicker();
    this.ticker = setInterval(() => {
      this.refresh().catch(e => log.error('refresh failed', describeError(e)));
    }, config.viewRefreshIntervalMs);
  }

  private stopTicker() {
    if (this.ticker) {
      clearInterval(this.ticker);
      this.ticker = null;
    }
  }

  private maxLamport(): number {
    return this.envelopes.reduce((max, e) => Math.max(max, e.lamport), this.lamportFloor);
  }

  private async pruneExpired(): Promise<void> {
    const now = this.now();
    const floor = this.maxLamport();
    this.envelopes = this.envelopes.filter(e => !isExpired(e, now));
    for (const [id, env] of this.carrying) {
      if (isExpired(env, now)) {
        this.carrying.delete(id);
      }
    }
    await this.repo.pruneExpired(now);
    if (floor !== this.lamportFloor) {
      this.lamportFloor = floor;
      await this.repo.saveLocalState(this.localState());
    }
  }

  async loadSampleHousehold(): Promise<ActionResult> {
    const sample = sampleSnapshot(this.now());
    return this.adopt(sample.household, sample.device, sample.envelopes);
  }

  async createHousehold(householdName: string, yourName: string): Promise<ActionResult> {
    const name = householdName.trim();
    const you = yourName.trim();
    if (!name || !you) {
      return {ok: false, error: 'Please enter a circle name and your name.'};
    }
    const personId = devUuid();
    const household: Household = {
      id: devUuid(),
      name,
      members: [{id: personId, display_name: you, role: 'adult'}],
    };
    return this.adopt(household, {deviceId: devUuid(), personId}, []);
  }

  private async adopt(
    household: Household,
    device: DeviceProfile,
    envelopes: Envelope[],
  ): Promise<ActionResult> {
    try {
      await this.repo.saveHousehold(household, device);
      await this.repo.addEnvelopes(envelopes);
      await this.repo.clearCarried();
      this.envelopes = envelopes;
      this.resetLocalState();
      await this.repo.saveLocalState(this.localState());
      this.update({
        phase: 'ready',
        household,
        device,
        relayEnabled: true,
        activity: [],
        error: null,
      });
      this.refreshView();
      this.startTransport();
      this.startTicker();
      return {ok: true};
    } catch (e) {
      return this.fail('Could not save your circle', e);
    }
  }

  private resetLocalState() {
    this.carrying.clear();
    this.joined.clear();
    this.rosters.clear();
    this.issuedInvites = [];
    this.blocked.clear();
    this.relayEnabled = true;
    this.peerHoldings.clear();
    this.lamportFloor = 0;
  }

  private restoreLocalState(local: LocalState, carried: Envelope[]) {
    this.resetLocalState();
    local.neighborhoods.forEach(n => this.joined.set(n.id, n));
    local.blocked.forEach(p => this.blocked.add(p));
    this.relayEnabled = local.relayEnabled;
    this.lamportFloor = local.lamport ?? 0;
    // Invites only matter for today's limit; older ones need not be kept.
    const now = this.now();
    this.issuedInvites = local.issuedInvites.filter(i => now - i.at < DAY_MS);
    if (this.relayEnabled) {
      carried.forEach(e => this.carrying.set(e.id, e));
    }
  }

  private localState(): LocalState {
    return {
      neighborhoods: [...this.joined.values()],
      blocked: [...this.blocked],
      relayEnabled: this.relayEnabled,
      issuedInvites: this.issuedInvites,
      lamport: this.lamportFloor,
    };
  }

  /** Save local-only state. Failures are surfaced but never block the UI. */
  private async persistLocal(): Promise<void> {
    try {
      await this.repo.saveLocalState(this.localState());
    } catch (e) {
      this.fail('Could not save your settings', e);
    }
  }

  async reset(): Promise<void> {
    this.transport.stop();
    this.stopTicker();
    await this.repo.clear();
    this.envelopes = [];
    this.resetLocalState();
    this.update({
      phase: 'onboarding',
      household: null,
      device: null,
      view: EMPTY_VIEW,
      envelopeCount: 0,
      neighborhoods: [],
      inbox: [],
      people: {},
      blocked: [],
      relayEnabled: true,
      carryingForOthers: 0,
      nearby: [],
      activity: [],
      error: null,
    });
  }

  // --- who's who ---

  private me(): Uuid | null {
    return this.snapshot.device?.personId ?? null;
  }

  private myName(): string {
    const {household, device} = this.snapshot;
    return household?.members.find(m => m.id === device?.personId)?.display_name ?? 'Me';
  }

  private readerContext(): ReaderContext | null {
    const {household, device} = this.snapshot;
    if (!household || !device) {
      return null;
    }
    return {
      localPersonId: device.personId,
      household,
      inNeighborhood: id => this.joined.has(id),
      isNeighborhoodMember: (id, person) => this.rosters.get(id)?.members.has(person) ?? false,
      knows: person =>
        household.members.some(m => m.id === person) ||
        [...this.rosters.values()].some(r => r.members.has(person)),
    };
  }

  /** Could this phone read `env`? (In Rust: does it hold the right key?) */
  private canRead(env: Envelope): boolean {
    const a = env.audience;
    const me = this.me();
    if (a.scope === 'household') {
      return a.household_id === this.snapshot.household?.id;
    }
    if (a.scope === 'direct') {
      return a.recipient === me;
    }
    // A removed member stops receiving new keys, so can't read new posts.
    const roster = this.rosters.get(a.neighborhood_id);
    return this.joined.has(a.neighborhood_id) && !(me && roster?.removed.has(me));
  }

  // --- household ---

  /**
   * Adds someone to the household on this device only. The real flow will
   * pair phones in person (QR code / tap) and exchange keys; see
   * docs/security.md. Membership is never discovered over the air.
   */
  async addMember(displayName: string, role: PersonRole): Promise<ActionResult> {
    const {household, device} = this.snapshot;
    const name = displayName.trim();
    if (!household || !device) {
      return {ok: false, error: 'Set up your circle first.'};
    }
    if (!name) {
      return {ok: false, error: 'Please enter a name.'};
    }
    const updated: Household = {
      ...household,
      members: [...household.members, {id: devUuid(), display_name: name, role}],
    };
    try {
      await this.repo.saveHousehold(updated, device);
      this.update({household: updated});
      this.refreshView();
      return {ok: true};
    } catch (e) {
      return this.fail('Could not add that person', e);
    }
  }

  checkIn(personId: Uuid, state: CheckInState, note?: string, rendezvousId?: Uuid) {
    const trimmed = note?.trim();
    return this.recordHousehold({
      type: 'check_in',
      data: {
        person_id: personId,
        state,
        note: trimmed || undefined,
        rendezvous_id: rendezvousId,
        recorded_at: this.now(),
      },
    });
  }

  saveRendezvous(
    input: Omit<RendezvousPoint, 'id' | 'household_id' | 'rank'> & {id?: Uuid; rank?: number},
  ) {
    const household = this.snapshot.household;
    if (!household) {
      return Promise.resolve<ActionResult>({ok: false, error: 'Set up your circle first.'});
    }
    return this.recordHousehold({
      type: 'rendezvous_upsert',
      data: {
        id: input.id ?? devUuid(),
        household_id: household.id,
        name: input.name.trim(),
        details: input.details.trim(),
        when_to_go: input.when_to_go.trim(),
        location: input.location,
        rank: input.rank ?? this.snapshot.view.rendezvous.length + 1,
      },
    });
  }

  removeRendezvous(id: Uuid) {
    return this.recordHousehold({type: 'rendezvous_remove', data: {id}});
  }

  postNote(text: string) {
    return this.recordHousehold({type: 'note', data: {text: text.trim()}});
  }

  private recordHousehold(payload: Payload) {
    const household = this.snapshot.household;
    if (!household) {
      return Promise.resolve<ActionResult>({ok: false, error: 'Set up your circle first.'});
    }
    return this.record({scope: 'household', household_id: household.id}, payload);
  }

  // --- neighborhoods ---

  async createNeighborhood(name: string, area: string): Promise<ActionResult> {
    const me = this.me();
    if (!me) {
      return {ok: false, error: 'Set up your circle first.'};
    }
    if (!name.trim()) {
      return {ok: false, error: 'Please name the neighborhood.'};
    }
    if (this.joined.size >= MAX_NEIGHBORHOODS) {
      return {ok: false, error: `You can be in up to ${MAX_NEIGHBORHOODS} neighborhoods.`};
    }
    const info: Neighborhood = {
      id: devUuid(),
      name: name.trim(),
      area: area.trim(),
      founder: me,
      created_at: this.now(),
    };
    return this.join(info, undefined, []);
  }

  /**
   * Development stand-in for joining with an invite scanned in person: a
   * simulated neighbor ("Dana") founded "Elm & Oak" and invites you; her
   * neighbor Marcus is already a member. Both become simulated phones.
   */
  async joinSampleNeighborhood(): Promise<ActionResult> {
    if (!this.me()) {
      return {ok: false, error: 'Set up your circle first.'};
    }
    if (this.joined.size >= MAX_NEIGHBORHOODS) {
      return {ok: false, error: `You can be in up to ${MAX_NEIGHBORHOODS} neighborhoods.`};
    }
    const now = this.now();
    const dana = devUuid();
    const marcus = devUuid();
    const info: Neighborhood = {
      id: devUuid(),
      name: 'Elm & Oak',
      area: 'Elm and Oak streets',
      founder: dana,
      created_at: now - 3 * DAY_MS,
    };
    // The signed roster Dana hands over along with the invite.
    const roster = [
      joinEnvelope(info.id, dana, 'Dana Okafor', undefined, now - 3 * DAY_MS),
      joinEnvelope(info.id, marcus, 'Marcus Webb', dana, now - 2 * DAY_MS),
    ];
    return this.join(info, dana, roster);
  }

  private async join(
    info: Neighborhood,
    invitedBy: Uuid | undefined,
    roster: Envelope[],
  ): Promise<ActionResult> {
    this.joined.set(info.id, info);
    if (roster.length) {
      await this.repo.addEnvelopes(roster);
      this.envelopes = [...this.envelopes, ...roster];
      this.refreshView();
    }
    const result = await this.record(
      {scope: 'neighborhood', neighborhood_id: info.id},
      {
        type: 'member_joined',
        data: {
          neighborhood_id: info.id,
          member: this.me()!,
          display_name: this.myName(),
          invited_by: invitedBy,
          invited_at: this.now(),
        },
      },
    );
    if (!result.ok) {
      this.joined.delete(info.id);
      this.refreshView();
      return result;
    }
    await this.persistLocal();
    return result;
  }

  private invitesUsedToday(neighborhood: Uuid): number {
    const me = this.me();
    const now = this.now();
    const members = new Set<Uuid>();
    this.rosters.get(neighborhood)?.members.forEach(c => {
      if (c.invited_by === me && now - c.invited_at < DAY_MS) {
        members.add(c.member);
      }
    });
    this.issuedInvites
      .filter(i => i.neighborhood === neighborhood && now - i.at < DAY_MS)
      .forEach(i => members.add(i.member));
    return members.size;
  }

  /**
   * Invite someone standing next to you. Real invites are a QR exchange that
   * needs camera support; in development this simulates the newcomer's phone
   * accepting and announcing itself, and the newcomer becomes a simulated
   * neighbor.
   */
  async inviteNeighbor(neighborhood: Uuid, displayName: string): Promise<ActionResult> {
    const me = this.me();
    const roster = this.rosters.get(neighborhood);
    const name = displayName.trim();
    if (!me || !roster?.members.has(me)) {
      return {ok: false, error: 'Only members can invite people.'};
    }
    if (!name) {
      return {ok: false, error: 'Please enter their name.'};
    }
    if (this.invitesUsedToday(neighborhood) >= INVITES_PER_DAY) {
      return {ok: false, error: `You can invite up to ${INVITES_PER_DAY} people a day.`};
    }
    const now = this.now();
    const member = devUuid();
    this.issuedInvites.push({neighborhood, member, at: now});
    await this.persistLocal();
    // Their phone accepts and announces the join; we're standing together,
    // so we receive it right away.
    const join = joinEnvelope(neighborhood, member, name, me, now);
    await this.repo.addEnvelopes([join]);
    this.envelopes = [...this.envelopes, join];
    this.refreshView();
    if ((this.rosters.get(neighborhood)?.members.size ?? 0) > SOFT_MEMBER_CAP) {
      log.warn('neighborhood is past the soft size cap; suggest splitting');
    }
    return {ok: true};
  }

  private postToNeighborhood(neighborhood: Uuid, payload: Payload): Promise<ActionResult> {
    const me = this.me();
    if (me && usedToday(this.envelopes, me, this.now(), isNeighborhoodPost) >= POSTS_PER_DAY) {
      return Promise.resolve({
        ok: false,
        error: `You can post up to ${POSTS_PER_DAY} times a day in a neighborhood.`,
      });
    }
    return this.record({scope: 'neighborhood', neighborhood_id: neighborhood}, payload);
  }

  postAlert(neighborhood: Uuid, category: AlertCategory, text: string, area?: string) {
    return this.postToNeighborhood(neighborhood, {
      type: 'alert',
      data: {category, text: text.trim(), area: area?.trim() || undefined},
    });
  }

  postOffer(neighborhood: Uuid, text: string) {
    return this.postToNeighborhood(neighborhood, {type: 'offer', data: {text: text.trim()}});
  }

  postNeed(neighborhood: Uuid, text: string) {
    return this.postToNeighborhood(neighborhood, {type: 'need', data: {text: text.trim()}});
  }

  /** Opt-in household status. Never includes individual members. */
  shareHouseholdStatus(neighborhood: Uuid, allSafe: boolean, text?: string) {
    const name = this.snapshot.household?.name ?? 'Our household';
    return this.postToNeighborhood(neighborhood, {
      type: 'household_status',
      data: {household_name: name, all_safe: allSafe, text: text?.trim() || undefined},
    });
  }

  withdrawPost(neighborhood: Uuid, envelopeId: Uuid) {
    return this.postToNeighborhood(neighborhood, {
      type: 'withdraw',
      data: {envelope_id: envelopeId},
    });
  }

  /** Founder only. */
  addSteward(neighborhood: Uuid, member: Uuid): Promise<ActionResult> {
    if (this.joined.get(neighborhood)?.founder !== this.me()) {
      return Promise.resolve({ok: false, error: 'Only the founder can appoint stewards.'});
    }
    return this.record(
      {scope: 'neighborhood', neighborhood_id: neighborhood},
      {type: 'steward_added', data: {member}},
    );
  }

  /**
   * Steward only. In the Rust core this also rotates the neighborhood key and
   * sends it to every remaining member, so the removed person can't read new
   * posts; the mirror models that through `canRead`.
   */
  removeMember(
    neighborhood: Uuid,
    member: Uuid,
    reason: RemovalReason,
    cascade: boolean,
  ): Promise<ActionResult> {
    const me = this.me();
    if (!me || !this.rosters.get(neighborhood)?.stewards.has(me)) {
      return Promise.resolve({ok: false, error: 'Only stewards can remove people.'});
    }
    if (member === this.joined.get(neighborhood)?.founder || member === me) {
      return Promise.resolve({ok: false, error: "The founder and you can't be removed."});
    }
    return this.record(
      {scope: 'neighborhood', neighborhood_id: neighborhood},
      {type: 'member_removed', data: {member, reason, cascade}},
    );
  }

  /** How many people `member` brought in, for the cascade question. */
  invitedCount(neighborhood: Uuid, member: Uuid): number {
    const roster = this.rosters.get(neighborhood);
    return roster ? invitedSubtree(roster, member).length : 0;
  }

  /** Who invited whom, back to the founder. Shown to stewards only. */
  chainOf(neighborhood: Uuid, member: Uuid): Uuid[] {
    const roster = this.rosters.get(neighborhood);
    return roster ? inviteChain(roster, member) : [member];
  }

  // --- direct ---

  private sendDirect(to: Uuid, payload: Payload): Promise<ActionResult> {
    const me = this.me();
    const isDirect = (e: Envelope) => e.audience.scope === 'direct';
    if (me && usedToday(this.envelopes, me, this.now(), isDirect) >= DIRECT_PER_DAY) {
      return Promise.resolve({
        ok: false,
        error: `You can send up to ${DIRECT_PER_DAY} private messages a day.`,
      });
    }
    return this.record({scope: 'direct', recipient: to}, payload);
  }

  sendMessage(to: Uuid, text: string) {
    return this.sendDirect(to, {type: 'message', data: {text: text.trim()}});
  }

  askAreYouOk(to: Uuid) {
    return this.sendDirect(to, {type: 'wellness_request', data: {text: 'Are you OK?'}});
  }

  replyWellness(to: Uuid, state: CheckInState) {
    return this.sendDirect(to, {type: 'wellness_reply', data: {state}});
  }

  // --- local settings ---

  /** Hide someone's posts and messages on this phone only. */
  block(person: Uuid): Promise<void> {
    this.blocked.add(person);
    this.refreshView();
    return this.persistLocal();
  }

  unblock(person: Uuid): Promise<void> {
    this.blocked.delete(person);
    this.refreshView();
    return this.persistLocal();
  }

  async setRelayEnabled(enabled: boolean): Promise<void> {
    this.relayEnabled = enabled;
    if (!enabled) {
      this.carrying.clear();
    }
    this.update({relayEnabled: enabled, carryingForOthers: this.carrying.size});
    try {
      if (!enabled) {
        await this.repo.clearCarried();
      }
      await this.repo.saveLocalState(this.localState());
    } catch (e) {
      this.fail('Could not save your settings', e);
    }
  }

  // --- recording ---

  private async record(audience: Audience, payload: Payload): Promise<ActionResult> {
    const {device} = this.snapshot;
    const ctx = this.readerContext();
    if (!device || !ctx) {
      return {ok: false, error: 'Set up your circle first.'};
    }
    const now = this.now();
    const lamport = this.maxLamport() + 1;
    const envelope = createEnvelope({
      audience,
      author: device.personId,
      origin_device: device.deviceId,
      lamport,
      now,
      payload,
    });
    // For a direct message, the recipient is the one who must accept it.
    const view: ReaderContext =
      audience.scope === 'direct'
        ? {
            ...ctx,
            localPersonId: audience.recipient,
            knows: p => p === device.personId || ctx.knows(p),
          }
        : ctx;
    const problem = validateEnvelope(envelope, view, now);
    if (problem) {
      log.warn('rejected local update', {problem, kind: payload.type});
      return {ok: false, error: friendlyValidation(problem)};
    }
    try {
      await this.repo.addEnvelopes([envelope]);
      this.envelopes = [...this.envelopes, envelope];
      this.refreshView();
      log.debug(`recorded ${payload.type} (${audience.scope})`, {id: envelope.id, lamport});
      return {ok: true};
    } catch (e) {
      return this.fail('Could not save that update', e);
    }
  }

  // --- transport ---

  setTransport(kind: TransportKind): void {
    if (kind === this.transport.kind) {
      return;
    }
    this.transport.stop();
    this.transport = this.makeTransport(kind);
    this.peerHoldings.clear();
    this.update({transport: this.describeTransport(), nearby: []});
    if (this.snapshot.phase === 'ready') {
      this.startTransport();
    }
  }

  /** Development helper: bring a simulated phone into range now. */
  simulateEncounter(kind?: 'family' | 'neighbor' | 'passer-by'): boolean {
    return this.transport instanceof MockTransport ? this.transport.encounterNext(kind) : false;
  }

  setSimulateNoSignal(value: boolean): void {
    this.update({simulateNoSignal: value});
  }

  private makeTransport(kind: TransportKind): SyncTransport {
    if (kind === 'mock') {
      return new MockTransport(
        () => {
          const {household, device, view} = this.snapshot;
          if (!household || !device) {
            return null;
          }
          const neighborhoods: MockNeighborhood[] = [...this.joined.keys()].map(id => ({
            id,
            members: [...(this.rosters.get(id)?.members.values() ?? [])].map(c => ({
              id: c.member,
              display_name: c.display_name,
            })),
          }));
          return {
            household,
            localPersonId: device.personId,
            rendezvous: view.rendezvous,
            neighborhoods,
          };
        },
        {exchangeIntervalMs: config.mockExchangeIntervalMs, now: this.now},
      );
    }
    return new NativeTransportStub(kind);
  }

  private describeTransport() {
    return {
      kind: this.transport.kind,
      label: this.transport.label,
      status: this.transport.status(),
    };
  }

  private startTransport() {
    this.transport.start({
      onPeersChanged: nearby => this.update({nearby}),
      onEncounter: encounter => {
        this.exchange(encounter).catch(e => log.error('exchange failed', describeError(e)));
      },
    });
    this.update({transport: this.describeTransport()});
  }

  /** Store-and-forward: give the peer what it lacks, take what we lack. */
  private async exchange(encounter: Encounter): Promise<void> {
    const me = this.me();
    if (!me || !this.readerContext()) {
      return;
    }
    const now = this.now();
    const address = encounter.peer.address;
    const known = this.peerHoldings.get(address) ?? new Set<Uuid>();
    encounter.peerHas.forEach(id => known.add(id));

    const forwardable = (env: Envelope) =>
      !known.has(env.id) &&
      !isExpired(env, now) &&
      // A direct message that reached us has arrived; don't spread it further.
      !(env.audience.scope === 'direct' && env.audience.recipient === me) &&
      !(env.audience.scope === 'neighborhood' && this.blocked.has(env.author));
    const outbound = [...this.envelopes, ...this.carrying.values()].filter(forwardable);

    const localHave = new Set([...this.envelopes.map(e => e.id), ...this.carrying.keys()]);
    const inbound = await encounter.exchange(outbound, localHave);
    outbound.forEach(e => known.add(e.id));

    const toCarry: Envelope[] = [];
    const readable: Envelope[] = [];
    for (const env of inbound) {
      known.add(env.id);
      if (isExpired(env, now) || localHave.has(env.id)) {
        continue;
      }
      if (this.canRead(env)) {
        readable.push(env);
      } else if (this.relayEnabled) {
        this.carrying.set(env.id, env);
        toCarry.push(env);
      }
    }
    this.peerHoldings.set(address, known);
    const carried = toCarry.length;
    if (carried) {
      await this.repo.addCarried(toCarry);
    }

    // Membership events first, so posts from people who joined in the same
    // batch are recognized.
    readable.sort(
      (a, b) => Number(isMembershipEvent(b.payload)) - Number(isMembershipEvent(a.payload)),
    );
    let received = 0;
    let rejected = 0;
    for (const env of readable) {
      const problem = validateEnvelope(env, this.readerContext()!, now) ?? this.overRateLimit(env);
      if (problem) {
        rejected += 1;
        log.debug(`rejected envelope from ${encounter.peer.label}`, {problem});
        continue;
      }
      const added = await this.repo.addEnvelopes([env]);
      if (added.length) {
        this.envelopes = [...this.envelopes, env];
        received += 1;
        if (isMembershipEvent(env.payload)) {
          this.refreshView();
        }
      }
    }
    this.refreshView();

    const record: ExchangeRecord = {
      at: now,
      peerLabel: encounter.peer.label,
      sent: outbound.length,
      received,
      carried,
      rejected,
    };
    log.info(`exchanged with ${record.peerLabel}`, record);
    this.update({activity: [record, ...this.snapshot.activity].slice(0, MAX_ACTIVITY)});
  }

  /** Per-author daily limits for neighborhood posts and direct messages. */
  private overRateLimit(env: Envelope): string | null {
    if (isNeighborhoodPost(env)) {
      return usedToday(this.envelopes, env.author, env.created_at, isNeighborhoodPost) >=
        POSTS_PER_DAY
        ? 'author is over the daily post limit'
        : null;
    }
    if (env.audience.scope === 'direct') {
      const isDirect = (e: Envelope) => e.audience.scope === 'direct';
      return usedToday(this.envelopes, env.author, env.created_at, isDirect) >= DIRECT_PER_DAY
        ? 'sender is over the daily message limit'
        : null;
    }
    return null;
  }

  private refreshView() {
    const now = this.now();
    const {household, device} = this.snapshot;
    const me = device?.personId;

    // Rosters first: membership decides which posts are visible.
    const inHood = (e: Envelope, id: Uuid) =>
      e.audience.scope === 'neighborhood' && e.audience.neighborhood_id === id;
    this.rosters = new Map();
    for (const [id, info] of this.joined) {
      const events = this.envelopes.filter(e => inHood(e, id) && isMembershipEvent(e.payload));
      this.rosters.set(id, buildRoster(info, events));
    }

    const people: Record<Uuid, string> = {};
    household?.members.forEach(m => (people[m.id] = m.display_name));
    this.rosters.forEach(r =>
      r.members.forEach(c => {
        people[c.member] = people[c.member] ?? c.display_name;
      }),
    );

    const postsLeft = me
      ? Math.max(0, POSTS_PER_DAY - usedToday(this.envelopes, me, now, isNeighborhoodPost))
      : 0;
    const neighborhoods: NeighborhoodSummary[] = [...this.joined.values()]
      .map(info => {
        const roster = this.rosters.get(info.id)!;
        const visible = (a: Uuid) => roster.members.has(a) && !this.blocked.has(a);
        return {
          info,
          members: [...roster.members.values()].sort((a, b) =>
            a.display_name.localeCompare(b.display_name),
          ),
          stewards: [...roster.stewards],
          isFounder: info.founder === me,
          isSteward: !!me && roster.stewards.has(me),
          removedMe: !!me && roster.removed.has(me),
          feed: buildFeed(
            this.envelopes.filter(e => inHood(e, info.id)),
            now,
            visible,
          ),
          invitesLeftToday: Math.max(0, INVITES_PER_DAY - this.invitesUsedToday(info.id)),
          postsLeftToday: postsLeft,
          overSoftCap: roster.members.size > SOFT_MEMBER_CAP,
        };
      })
      .sort((a, b) => a.info.name.localeCompare(b.info.name));

    const isDirect = (e: Envelope) => e.audience.scope === 'direct';
    this.update({
      view: buildView(this.envelopes, now),
      envelopeCount: this.envelopes.length,
      neighborhoods,
      inbox: buildInbox(this.envelopes, now).filter(m => !this.blocked.has(m.from)),
      directLeftToday: me
        ? Math.max(0, DIRECT_PER_DAY - usedToday(this.envelopes, me, now, isDirect))
        : 0,
      people,
      blocked: [...this.blocked],
      carryingForOthers: this.carrying.size,
    });
  }

  private fail(message: string, error: unknown): ActionResult {
    const detail = describeError(error);
    log.error(message, detail);
    this.update({error: `${message}. ${detail}`});
    return {ok: false, error: message};
  }
}

function joinEnvelope(
  neighborhood: Uuid,
  member: Uuid,
  displayName: string,
  invitedBy: Uuid | undefined,
  at: number,
): Envelope {
  return createEnvelope({
    audience: {scope: 'neighborhood', neighborhood_id: neighborhood},
    author: member,
    origin_device: devUuid(),
    lamport: 1,
    now: at,
    payload: {
      type: 'member_joined',
      data: {
        neighborhood_id: neighborhood,
        member,
        display_name: displayName,
        invited_by: invitedBy,
        invited_at: at,
      },
    },
  });
}

function isNeighborhoodPost(e: Envelope): boolean {
  return e.audience.scope === 'neighborhood' && !isMembershipEvent(e.payload);
}

function friendlyValidation(problem: string): string {
  if (problem.includes('too long')) {
    return 'That is a bit long. Please shorten it.';
  }
  if (problem === 'empty' || problem.includes('empty') || problem.includes('needs a name')) {
    return 'Please fill that in first.';
  }
  if (problem.includes('not a member of the neighborhood')) {
    return "You're no longer a member of that neighborhood.";
  }
  return `That update could not be saved (${problem}).`;
}
