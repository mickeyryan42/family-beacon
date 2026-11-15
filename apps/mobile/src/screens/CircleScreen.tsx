/**
 * "Add trusted circle": create the household, then add people to it.
 *
 * Adding someone here only records them on this phone. The production flow
 * pairs phones face to face (QR code or tap) so keys are exchanged in person
 * and nobody can be added remotely. That step is stubbed for now.
 */
import React, {useState} from 'react';
import {View} from 'react-native';
import {useNavigation} from '@react-navigation/native';
import type {NativeStackNavigationProp} from '@react-navigation/native-stack';

import {
  Body,
  Button,
  Card,
  EmptyState,
  Field,
  Heading,
  InlineError,
  Screen,
  Small,
} from '../components/ui';
import {ROLE_LABELS} from '../lib/format';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';
import type {PersonRole} from '../sync/protocol';
import type {RootStackParamList} from '../navigation/types';
import {spacing, useTheme} from '../theme/theme';

const ROLES: PersonRole[] = ['adult', 'child', 'caregiver'];

export function CircleScreen() {
  const {household} = useBeacon();
  return household ? <AddMember /> : <CreateCircle />;
}

function CreateCircle() {
  const store = useBeaconStore();
  const [circleName, setCircleName] = useState('');
  const [yourName, setYourName] = useState('');
  const [error, setError] = useState<string | null>(null);

  const create = async () => {
    const result = await store.createHousehold(circleName, yourName);
    setError(result.ok ? null : result.error);
  };

  return (
    <Screen>
      <Body muted>
        Your circle is the small group of people who can see your check-ins. Usually your household,
        plus anyone you'd call first in an emergency.
      </Body>
      <Field
        label="Circle name"
        placeholder="e.g. The Garcia family"
        value={circleName}
        onChangeText={setCircleName}
        returnKeyType="next"
      />
      <Field
        label="Your name"
        placeholder="What your family calls you"
        value={yourName}
        onChangeText={setYourName}
        returnKeyType="done"
        onSubmitEditing={create}
      />
      <InlineError message={error} />
      <Button label="Create circle" onPress={create} />
    </Screen>
  );
}

function AddMember() {
  const store = useBeaconStore();
  const navigation = useNavigation<NativeStackNavigationProp<RootStackParamList>>();
  const {household, device} = useBeacon();
  const {colors} = useTheme();
  const [name, setName] = useState('');
  const [role, setRole] = useState<PersonRole>('adult');
  const [error, setError] = useState<string | null>(null);
  const [added, setAdded] = useState<string | null>(null);

  const add = async () => {
    const trimmed = name.trim();
    const result = await store.addMember(trimmed, role);
    if (result.ok) {
      setName('');
      setRole('adult');
      setError(null);
      setAdded(`Added ${trimmed} to your circle.`);
    } else {
      setAdded(null);
      setError(result.error);
    }
  };

  // Reached from Settings there is a screen to go back to; straight after
  // creating the circle there isn't, so go to the main tabs instead.
  const done = () => {
    if (navigation.canGoBack()) {
      navigation.goBack();
    } else {
      navigation.reset({index: 0, routes: [{name: 'Main'}]});
    }
  };

  const members = household?.members ?? [];

  return (
    <Screen footer={<Button label="Done" onPress={done} />}>
      <Heading>Add someone to {household?.name}</Heading>
      <Field
        label="Name"
        placeholder="e.g. Grandpa Joe"
        value={name}
        onChangeText={text => {
          setName(text);
          setAdded(null);
        }}
        returnKeyType="done"
        onSubmitEditing={add}
      />
      <Small>They are</Small>
      <View style={{flexDirection: 'row', gap: spacing.sm}}>
        {ROLES.map(r => (
          <View key={r} style={{flex: 1}}>
            <Button
              variant={role === r ? 'primary' : 'secondary'}
              label={ROLE_LABELS[r]}
              accessibilityHint={`Add them as ${ROLE_LABELS[r].toLowerCase()}`}
              onPress={() => setRole(r)}
              compact
            />
          </View>
        ))}
      </View>
      <Button label="Add to circle" onPress={add} disabled={!name.trim()} />
      <InlineError message={error} />
      {added ? <Small color={colors.status.safe}>{added}</Small> : null}

      <Heading>In your circle ({members.length})</Heading>
      {members.length <= 1 ? (
        <EmptyState
          title="Just you so far"
          body="Add the people who should see your check-ins. Children without phones can be added too; an adult can check in for them."
        />
      ) : null}
      {members.map(m => (
        <Card key={m.id}>
          <Body>
            {m.display_name}
            {m.id === device?.personId ? ' (you)' : ''}
          </Body>
          <Small>{ROLE_LABELS[m.role]}</Small>
        </Card>
      ))}
      <Card>
        <Body>Pairing phones in person</Body>
        <Small>
          Coming soon: hold two phones together to add someone securely. Nobody can join your circle
          remotely.
        </Small>
      </Card>
    </Screen>
  );
}
