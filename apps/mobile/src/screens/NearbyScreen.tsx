/**
 * "Nearby status / local updates": who is in range, what was exchanged, and
 * short notes shared with the circle (e.g. "Power's out on Elm St").
 */
import React, {useState} from 'react';
import {View} from 'react-native';

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
  Small,
} from '../components/ui';
import {timeAgo} from '../lib/format';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';
import {spacing, useTheme} from '../theme/theme';

const SIMULATIONS = [
  {kind: 'family', label: 'Family'},
  {kind: 'neighbor', label: 'Neighbor'},
  {kind: 'passer-by', label: 'Passer-by'},
] as const;

export function NearbyScreen() {
  const store = useBeaconStore();
  const {household, nearby, activity, view, transport, relayEnabled, carryingForOthers} =
    useBeacon();
  const {colors} = useTheme();
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);

  const nameOf = (id: string) =>
    household?.members.find(m => m.id === id)?.display_name ?? 'Someone';

  const post = async () => {
    const result = await store.postNote(text);
    if (result.ok) {
      setText('');
      setError(null);
    } else {
      setError(result.error);
    }
  };

  return (
    <Screen>
      <Heading>In range now</Heading>
      {nearby.length === 0 ? (
        <Card>
          <Body>No family phones nearby</Body>
          <Small>
            {transport.status.state === 'unavailable'
              ? transport.status.reason
              : 'Your updates are saved and will be shared automatically when someone is close by.'}
          </Small>
        </Card>
      ) : (
        nearby.map(p => (
          <Card key={p.address}>
            <Body>{p.label}</Body>
            <Small color={colors.status.safe}>Sharing updates</Small>
          </Card>
        ))
      )}
      {transport.kind === 'mock' ? (
        <Card>
          <Small>Development: bring a simulated phone into range now.</Small>
          <View style={{flexDirection: 'row', flexWrap: 'wrap', gap: spacing.sm}}>
            {SIMULATIONS.map(s => (
              <View key={s.kind} style={{flexGrow: 1, minWidth: '30%'}}>
                <Button
                  compact
                  variant="secondary"
                  label={s.label}
                  accessibilityHint="Development only. Runs one mock exchange now."
                  onPress={() => store.simulateEncounter(s.kind)}
                />
              </View>
            ))}
          </View>
        </Card>
      ) : null}
      {relayEnabled ? (
        <Small>
          Carrying {carryingForOthers} sealed {carryingForOthers === 1 ? 'message' : 'messages'} for
          other households. Your phone can't read them; it just passes them on.
        </Small>
      ) : null}

      <Heading>Family notes</Heading>
      <ScopeBadge
        scope="household"
        label="Family only. To tell neighbors, use the Neighbors tab."
      />
      <Field
        label="Share a short note with your family"
        placeholder="e.g. Power's out on our street"
        value={text}
        onChangeText={setText}
        maxLength={280}
      />
      <InlineError message={error} />
      <Button label="Share note" onPress={post} />
      {view.notes.length === 0 ? (
        <EmptyState
          title="No notes yet"
          body="Notes from your circle about what's happening nearby show up here."
        />
      ) : (
        view.notes.map(n => (
          <Card key={n.envelopeId}>
            <Body>{n.text}</Body>
            <Small>
              {nameOf(n.author)} · {timeAgo(n.createdAt)}
            </Small>
          </Card>
        ))
      )}

      <Heading>Recent exchanges</Heading>
      {activity.length === 0 ? (
        <Small>Nothing exchanged yet.</Small>
      ) : (
        activity.map(a => (
          <Small key={`${a.at}-${a.peerLabel}`}>
            {timeAgo(a.at)}: {a.peerLabel}. Sent {a.sent}, received {a.received}
            {a.carried ? `, carrying ${a.carried} for others` : ''}
            {a.rejected ? `, ignored ${a.rejected}` : ''}.
          </Small>
        ))
      )}
    </Screen>
  );
}
