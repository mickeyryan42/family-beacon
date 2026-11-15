/**
 * Neighborhood: an invite-only network of nearby households.
 *
 * Everything posted here is clearly labelled with who will see it, and family
 * check-ins never appear here. See docs/neighborhood-network.md.
 */
import React, {useMemo, useState} from 'react';
import {View} from 'react-native';
import type {BottomTabScreenProps} from '@react-navigation/bottom-tabs';
import type {CompositeScreenProps} from '@react-navigation/native';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {
  Body,
  Button,
  Card,
  EmptyState,
  Field,
  Heading,
  InlineError,
  Screen,
  ScopeBadge,
  Segmented,
  Small,
  StatusDot,
} from '../components/ui';
import {config} from '../config';
import {ALERT_LABELS, DELIVERY_NOTE, timeAgo} from '../lib/format';
import type {RootStackParamList, TabParamList} from '../navigation/types';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';
import type {ActionResult, NeighborhoodSummary} from '../state/beaconStore';
import type {Post} from '../sync/neighborhoodView';
import {MAX_NEIGHBORHOODS} from '../sync/neighborhoodView';
import type {AlertCategory, Uuid} from '../sync/protocol';
import {spacing, useTheme} from '../theme/theme';

type Props = CompositeScreenProps<
  BottomTabScreenProps<TabParamList, 'Neighbors'>,
  NativeStackScreenProps<RootStackParamList>
>;

type ComposeKind = 'alert' | 'offer' | 'need' | 'status';
const COMPOSE_OPTIONS: {value: ComposeKind; label: string}[] = [
  {value: 'alert', label: 'Alert'},
  {value: 'offer', label: 'Offer'},
  {value: 'need', label: 'Need'},
  {value: 'status', label: 'All safe'},
];
const CATEGORIES = Object.keys(ALERT_LABELS) as AlertCategory[];

export function NeighborhoodScreen({navigation}: Props) {
  const store = useBeaconStore();
  const {neighborhoods} = useBeacon();
  const [selected, setSelected] = useState<Uuid | null>(null);
  const [adding, setAdding] = useState(false);

  const current = neighborhoods.find(n => n.info.id === selected) ?? neighborhoods[0];

  if (!current || adding) {
    return (
      <Screen>
        <JoinOrCreate
          onDone={() => {
            // Switch to whichever neighborhood was just joined or started.
            const before = new Set(neighborhoods.map(n => n.info.id));
            const added = store.getSnapshot().neighborhoods.find(n => !before.has(n.info.id));
            setAdding(false);
            setSelected(added?.info.id ?? null);
          }}
          onCancel={neighborhoods.length ? () => setAdding(false) : undefined}
        />
      </Screen>
    );
  }

  return (
    <Screen>
      {neighborhoods.length > 1 ? (
        <Segmented
          options={neighborhoods.map(n => ({value: n.info.id, label: n.info.name}))}
          value={current.info.id}
          onChange={setSelected}
        />
      ) : null}
      <Card>
        <Heading>{current.info.name}</Heading>
        {current.info.area ? <Small>{current.info.area}</Small> : null}
        <Button
          variant="secondary"
          label={`People (${current.members.length})`}
          onPress={() =>
            navigation.navigate('NeighborhoodPeople', {neighborhoodId: current.info.id})
          }
        />
      </Card>
      {current.removedMe ? (
        <Card>
          <Body>You were removed from this neighborhood</Body>
          <Small>You can no longer read or post new updates here.</Small>
        </Card>
      ) : (
        <Composer hood={current} />
      )}
      <Feed hood={current} onMessage={id => navigation.navigate('Conversation', {personId: id})} />
      <Messages onOpen={id => navigation.navigate('Conversation', {personId: id})} />
      {neighborhoods.length < MAX_NEIGHBORHOODS ? (
        <Button
          variant="secondary"
          label="Join or start another neighborhood"
          onPress={() => setAdding(true)}
        />
      ) : null}
    </Screen>
  );
}

function JoinOrCreate({onDone, onCancel}: {onDone: () => void; onCancel?: () => void}) {
  const store = useBeaconStore();
  const [name, setName] = useState('');
  const [area, setArea] = useState('');
  const [error, setError] = useState<string | null>(null);

  const finish = (result: ActionResult) => {
    if (result.ok) {
      onDone();
    } else {
      setError(result.error);
    }
  };

  return (
    <>
      <Heading>Your neighborhood</Heading>
      <Body muted>
        Connect with nearby households to share outage news, offer help, and check on each other,
        all without signal.
      </Body>
      <Card>
        <Body>What neighbors can see</Body>
        <Small>
          Posts you share with the neighborhood, your name, and who invited you. Every neighbor's
          phone also helps carry sealed messages, which it can't read, further along.
        </Small>
        <Body>What they never see</Body>
        <Small>
          Your family's check-ins, meeting points and notes. Those stay with your family.
        </Small>
      </Card>

      <Heading>Join with an invite</Heading>
      <Card>
        <Small>
          Ask a neighbor who's already a member to invite you. You'll scan their code in person.
          Scanning arrives with camera support.
        </Small>
        {config.offerSampleHousehold ? (
          <Button
            variant="secondary"
            label="Simulate joining Elm & Oak"
            accessibilityHint="Development only. Joins a sample neighborhood with simulated neighbors."
            onPress={async () => finish(await store.joinSampleNeighborhood())}
          />
        ) : null}
      </Card>

      <Heading>Or start one</Heading>
      <Field
        label="Neighborhood name"
        placeholder="e.g. Elm & Oak"
        value={name}
        onChangeText={setName}
        maxLength={80}
      />
      <Field
        label="Area, in words"
        placeholder="e.g. Elm and Oak streets"
        value={area}
        onChangeText={setArea}
        maxLength={80}
      />
      <Small>You'll be the founder and first steward. Stewards can remove people if needed.</Small>
      <InlineError message={error} />
      <Button
        label="Start neighborhood"
        disabled={!name.trim()}
        onPress={async () => finish(await store.createNeighborhood(name, area))}
      />
      {onCancel ? <Button variant="secondary" label="Cancel" onPress={onCancel} /> : null}
    </>
  );
}

function Composer({hood}: {hood: NeighborhoodSummary}) {
  const store = useBeaconStore();
  const {household} = useBeacon();
  const {colors} = useTheme();
  const [kind, setKind] = useState<ComposeKind>('alert');
  const [category, setCategory] = useState<AlertCategory>('power');
  const [text, setText] = useState('');
  const [area, setArea] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [sent, setSent] = useState<string | null>(null);

  const id = hood.info.id;
  const submit = async () => {
    const result =
      kind === 'alert'
        ? await store.postAlert(id, category, text, area)
        : kind === 'offer'
        ? await store.postOffer(id, text)
        : kind === 'need'
        ? await store.postNeed(id, text)
        : await store.shareHouseholdStatus(id, true, text);
    if (result.ok) {
      setText('');
      setArea('');
      setError(null);
      setSent(`Shared with ${hood.info.name}.`);
    } else {
      setSent(null);
      setError(result.error);
    }
  };

  const placeholder = {
    alert: 'e.g. Transformer down at Elm & 5th',
    offer: 'e.g. Generator here, come charge phones',
    need: 'e.g. Need ice for insulin',
    status: 'Optional note, e.g. Staying at home tonight',
  }[kind];

  return (
    <Card>
      <ScopeBadge
        scope="neighborhood"
        label={`Shared with ${hood.info.name} · ${hood.members.length} people`}
      />
      <Segmented options={COMPOSE_OPTIONS} value={kind} onChange={setKind} />
      {kind === 'alert' ? (
        <Segmented
          options={CATEGORIES.map(c => ({value: c, label: ALERT_LABELS[c]}))}
          value={category}
          onChange={setCategory}
        />
      ) : null}
      {kind === 'status' ? (
        <View style={{gap: spacing.xs}}>
          <Small>Neighbors will see exactly this:</Small>
          <Body>{household?.name ?? 'Your household'}: everyone safe</Body>
          <Small>Individual names, check-ins and locations are never included.</Small>
        </View>
      ) : null}
      <Field
        label={kind === 'status' ? 'Note (optional)' : 'What is happening?'}
        placeholder={placeholder}
        value={text}
        onChangeText={t => {
          setText(t);
          setSent(null);
        }}
        maxLength={280}
        multiline
      />
      {kind === 'alert' ? (
        <Field
          label="Where, roughly (optional)"
          placeholder="e.g. Elm St, 400 block"
          value={area}
          onChangeText={setArea}
          maxLength={80}
        />
      ) : null}
      <InlineError message={error} />
      {sent ? <Small color={colors.status.safe}>{sent}</Small> : null}
      <Button
        label={kind === 'status' ? 'Share "everyone safe"' : 'Share with neighborhood'}
        disabled={kind !== 'status' && !text.trim()}
        onPress={submit}
      />
      <Small>
        {DELIVERY_NOTE} {hood.postsLeftToday} posts left today.
      </Small>
    </Card>
  );
}

function postTitle(post: Post): string {
  switch (post.kind) {
    case 'alert':
      return `${ALERT_LABELS[post.category]} alert`;
    case 'offer':
      return 'Offer';
    case 'need':
      return 'Need';
    case 'household_status':
      return post.all_safe ? 'Everyone safe' : 'Household update';
  }
}

function postBody(post: Post): string | undefined {
  if (post.kind === 'household_status') {
    return [post.household_name, post.text].filter(Boolean).join('. ');
  }
  return post.text;
}

function Feed({hood, onMessage}: {hood: NeighborhoodSummary; onMessage: (id: Uuid) => void}) {
  const store = useBeaconStore();
  const {device, people} = useBeacon();
  const {colors} = useTheme();

  if (hood.feed.length === 0) {
    return (
      <EmptyState
        title="Quiet for now"
        body="Alerts, offers and needs from neighbors show up here as phones pass each other."
      />
    );
  }

  const urgent = (p: Post) =>
    p.kind === 'alert' && (p.category === 'medical' || p.category === 'fire');

  return (
    <>
      <Heading>Latest</Heading>
      {hood.feed.map(post => {
        const mine = post.author === device?.personId;
        return (
          <Card key={post.envelopeId}>
            <View style={{flexDirection: 'row', alignItems: 'center', gap: spacing.sm}}>
              <StatusDot color={urgent(post) ? colors.status.need_help : colors.status.on_my_way} />
              <Body>{postTitle(post)}</Body>
            </View>
            {postBody(post) ? <Body>{postBody(post)}</Body> : null}
            {post.kind === 'alert' && post.area ? <Small>Where: {post.area}</Small> : null}
            <Small>
              {mine ? 'You' : people[post.author] ?? 'A neighbor'} · {timeAgo(post.createdAt)}
            </Small>
            {mine ? (
              <Button
                variant="secondary"
                label="Withdraw"
                onPress={() => store.withdrawPost(hood.info.id, post.envelopeId)}
              />
            ) : (
              <View style={{flexDirection: 'row', gap: spacing.sm}}>
                <View style={{flex: 1}}>
                  <Button
                    compact
                    variant="secondary"
                    label="Message"
                    onPress={() => onMessage(post.author)}
                  />
                </View>
                <View style={{flex: 1}}>
                  <Button
                    compact
                    variant="secondary"
                    label="Block"
                    onPress={() => store.block(post.author)}
                  />
                </View>
              </View>
            )}
          </Card>
        );
      })}
    </>
  );
}

function Messages({onOpen}: {onOpen: (id: Uuid) => void}) {
  const {inbox, device, people} = useBeacon();
  const me = device?.personId;

  const threads = useMemo(() => {
    const latest = new Map<Uuid, (typeof inbox)[number]>();
    for (const m of inbox) {
      latest.set(m.from === me ? m.to : m.from, m);
    }
    return [...latest.entries()].sort((a, b) => b[1].createdAt - a[1].createdAt);
  }, [inbox, me]);

  if (threads.length === 0) {
    return null;
  }
  return (
    <>
      <Heading>Private messages</Heading>
      {threads.map(([other, last]) => {
        const preview =
          last.kind === 'message'
            ? last.text
            : last.kind === 'wellness_request'
            ? 'Asked: Are you OK?'
            : last.state === 'need_help'
            ? 'Needs help'
            : 'Is safe';
        const waiting = last.kind === 'wellness_request' && last.from !== me;
        return (
          <Card key={other}>
            <Body>{people[other] ?? 'A neighbor'}</Body>
            <Small>
              {last.from === me ? 'You: ' : ''}
              {preview} · {timeAgo(last.createdAt)}
              {waiting ? ' · waiting for your answer' : ''}
            </Small>
            <Button variant="secondary" label="Open" onPress={() => onOpen(other)} />
          </Card>
        );
      })}
    </>
  );
}
