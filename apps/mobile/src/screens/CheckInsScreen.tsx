import React, {useState} from 'react';
import {View} from 'react-native';
import type {BottomTabScreenProps} from '@react-navigation/bottom-tabs';
import type {CompositeScreenProps} from '@react-navigation/native';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {
  Body,
  Button,
  Card,
  ChoiceRow,
  EmptyState,
  Field,
  Heading,
  InlineError,
  Screen,
  Small,
  ScopeBadge,
  StatusDot,
} from '../components/ui';
import {CHECK_IN_LABELS, timeAgo} from '../lib/format';
import type {RootStackParamList, TabParamList} from '../navigation/types';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';
import type {CheckInState, Uuid} from '../sync/protocol';
import {spacing, useTheme} from '../theme/theme';

type Props = CompositeScreenProps<
  BottomTabScreenProps<TabParamList, 'CheckIns'>,
  NativeStackScreenProps<RootStackParamList>
>;

const STATES: CheckInState[] = ['safe', 'on_my_way', 'at_rendezvous', 'need_help'];

export function CheckInsScreen({navigation}: Props) {
  const store = useBeaconStore();
  const {household, device, view} = useBeacon();
  const {colors} = useTheme();
  const [subject, setSubject] = useState<Uuid | null>(null);
  const [note, setNote] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  if (!household || !device) {
    return null;
  }
  const members = household.members;
  const nameOf = (id: Uuid) => members.find(m => m.id === id)?.display_name ?? 'Someone';
  const forPerson = subject ?? device.personId;
  const primaryPoint = view.rendezvous[0];

  const send = async (state: CheckInState) => {
    const rendezvousId =
      state === 'at_rendezvous' || state === 'on_my_way' ? primaryPoint?.id : undefined;
    const result = await store.checkIn(forPerson, state, note, rendezvousId);
    if (result.ok) {
      setNote('');
      setError(null);
      setSaved('Saved. It will reach family phones as they come nearby.');
    } else {
      setSaved(null);
      setError(result.error);
    }
  };

  return (
    <Screen>
      <Heading>
        {forPerson === device.personId ? 'How are you?' : `Checking in for ${nameOf(forPerson)}`}
      </Heading>
      <ScopeBadge scope="household" label="Family only. Neighbors never see check-ins." />
      <View style={{gap: spacing.sm}}>
        {STATES.map(state => (
          <ChoiceRow
            key={state}
            label={CHECK_IN_LABELS[state]}
            detail={
              state === 'at_rendezvous' && primaryPoint
                ? primaryPoint.name
                : state === 'need_help'
                ? 'Sent first, ahead of everything else'
                : undefined
            }
            dotColor={colors.status[state]}
            onPress={() => send(state)}
          />
        ))}
      </View>
      <Field
        label="Add a short note (optional)"
        placeholder="e.g. At the library with Sam"
        value={note}
        onChangeText={setNote}
        maxLength={280}
      />
      {members.length > 1 ? (
        <View style={{flexDirection: 'row', flexWrap: 'wrap', gap: spacing.sm}}>
          {members.map(m => (
            <View key={m.id} style={{minWidth: '30%', flexGrow: 1}}>
              <Button
                variant={m.id === forPerson ? 'primary' : 'secondary'}
                label={m.id === device.personId ? 'Me' : m.display_name}
                accessibilityHint={`Check in on behalf of ${m.display_name}`}
                onPress={() => setSubject(m.id)}
              />
            </View>
          ))}
        </View>
      ) : null}
      <InlineError message={error} />
      {saved ? <Small color={colors.status.safe}>{saved}</Small> : null}

      <Heading>Family</Heading>
      {members.length <= 1 ? (
        <EmptyState
          title="Add your family"
          body="Check-ins from your circle will show here once you add people."
          action={
            <Button
              variant="secondary"
              label="Add people"
              onPress={() => navigation.navigate('Circle')}
            />
          }
        />
      ) : (
        members.map(m => {
          const latest = view.checkIns[m.id];
          return (
            <Card key={m.id}>
              <View style={{flexDirection: 'row', alignItems: 'center', gap: spacing.sm}}>
                <StatusDot color={latest ? colors.status[latest.state] : colors.status.unknown} />
                <View style={{flex: 1}}>
                  <Body>{m.display_name}</Body>
                  <Small>
                    {latest
                      ? `${CHECK_IN_LABELS[latest.state]} · ${timeAgo(latest.recorded_at)}`
                      : 'No check-in yet'}
                  </Small>
                </View>
              </View>
              {latest?.note ? <Body muted>{`“${latest.note}”`}</Body> : null}
            </Card>
          );
        })
      )}
    </Screen>
  );
}
