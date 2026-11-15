import React, {useState} from 'react';
import {Alert, View} from 'react-native';
import type {BottomTabScreenProps} from '@react-navigation/bottom-tabs';
import type {CompositeScreenProps} from '@react-navigation/native';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {Body, Button, Card, ChoiceRow, Heading, Screen, Small, ToggleRow} from '../components/ui';
import {recentLogs} from '../lib/logger';
import type {RootStackParamList, TabParamList} from '../navigation/types';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';
import type {TransportKind} from '../sync/transport';
import {spacing, useTheme, type ThemeMode} from '../theme/theme';

type Props = CompositeScreenProps<
  BottomTabScreenProps<TabParamList, 'Settings'>,
  NativeStackScreenProps<RootStackParamList>
>;

const THEMES: {mode: ThemeMode; label: string}[] = [
  {mode: 'dark', label: 'Dark'},
  {mode: 'light', label: 'Light'},
  {mode: 'system', label: 'Match my phone'},
];

const TRANSPORTS: {kind: TransportKind; label: string; detail: string}[] = [
  {kind: 'mock', label: 'Mock transport', detail: 'Simulated family phones. No radios used.'},
  {kind: 'ble', label: 'Bluetooth LE', detail: 'Not built yet'},
  {kind: 'wifi-direct', label: 'Wi-Fi Direct', detail: 'Not built yet'},
];

export function SettingsScreen({navigation}: Props) {
  const store = useBeaconStore();
  const {
    household,
    envelopeCount,
    transport,
    simulateNoSignal,
    relayEnabled,
    carryingForOthers,
    blocked,
    people,
  } = useBeacon();
  const theme = useTheme();
  const [showLogs, setShowLogs] = useState(false);

  const confirmReset = () =>
    Alert.alert(
      'Remove all data from this phone?',
      'Your circle and every saved update will be deleted.',
      [
        {text: 'Cancel', style: 'cancel'},
        {text: 'Remove', style: 'destructive', onPress: () => store.reset()},
      ],
    );

  return (
    <Screen>
      <Heading>Circle</Heading>
      <Card>
        <Body>{household?.name}</Body>
        <Small>
          {household?.members.length ?? 0} people · {envelopeCount} updates stored on this phone
        </Small>
        <Button
          variant="secondary"
          label="Manage people"
          onPress={() => navigation.navigate('Circle')}
        />
      </Card>

      <Heading>Appearance</Heading>
      <View style={{gap: spacing.sm}}>
        {THEMES.map(t => (
          <ChoiceRow
            key={t.mode}
            label={t.label}
            selected={theme.mode === t.mode}
            onPress={() => theme.setMode(t.mode)}
          />
        ))}
      </View>

      <Heading>Privacy</Heading>
      <Card>
        <Small>
          Everything stays on this phone and the phones of people in your circle. Family Beacon has
          no account, no cloud, and no tracking.
        </Small>
      </Card>

      <Heading>Helping neighbors</Heading>
      <ToggleRow
        label="Help carry neighbors' sealed messages"
        detail={`Your phone passes on messages it can't read, so updates travel further. Uses a little extra battery.${
          relayEnabled ? ` Carrying ${carryingForOthers} right now.` : ''
        }`}
        value={relayEnabled}
        onValueChange={v => store.setRelayEnabled(v)}
      />
      {blocked.length > 0 ? (
        <Card>
          <Body>Blocked on this phone</Body>
          {blocked.map(id => (
            <View key={id} style={{flexDirection: 'row', alignItems: 'center', gap: spacing.sm}}>
              <View style={{flex: 1}}>
                <Small>{people[id] ?? 'Someone'}</Small>
              </View>
              <Button
                compact
                variant="secondary"
                label="Unblock"
                onPress={() => store.unblock(id)}
              />
            </View>
          ))}
        </Card>
      ) : null}

      {__DEV__ ? (
        <>
          <Heading>Developer</Heading>
          <View style={{gap: spacing.sm}}>
            {TRANSPORTS.map(t => (
              <ChoiceRow
                key={t.kind}
                label={t.label}
                detail={t.detail}
                selected={transport.kind === t.kind}
                onPress={() => store.setTransport(t.kind)}
              />
            ))}
          </View>
          <ToggleRow
            label="Simulate no signal"
            detail="Shows the app as it looks with no cellular or internet."
            value={simulateNoSignal}
            onValueChange={v => store.setSimulateNoSignal(v)}
          />
          <Button
            variant="secondary"
            label={showLogs ? 'Hide recent logs' : 'Show recent logs'}
            onPress={() => setShowLogs(s => !s)}
          />
          {showLogs ? (
            <Card>
              {recentLogs()
                .slice(-30)
                .reverse()
                .map((l, i) => (
                  <Small key={`${l.at}-${i}`}>
                    {l.level.toUpperCase()} [{l.tag}] {l.message}
                  </Small>
                ))}
            </Card>
          ) : null}
        </>
      ) : null}

      <Heading>Data</Heading>
      <Button variant="danger" label="Remove all data from this phone" onPress={confirmReset} />
    </Screen>
  );
}
