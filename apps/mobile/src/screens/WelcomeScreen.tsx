import React, {useState} from 'react';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {Body, Button, Card, Gap, InlineError, Screen, Small, Title} from '../components/ui';
import {config} from '../config';
import type {RootStackParamList} from '../navigation/types';
import {useBeaconStore} from '../state/BeaconProvider';

type Props = NativeStackScreenProps<RootStackParamList, 'Welcome'>;

const POINTS = [
  ['Check in with one tap', "Let your family know you're safe, on your way, or need help."],
  ['Agree where to meet', 'Save meeting points ahead of time, so everyone knows the plan.'],
  ['Works with no signal', 'Updates pass phone to phone when family members are near each other.'],
  ['Private by design', 'Only people you add in person. Nothing is stored in the cloud.'],
] as const;

export function WelcomeScreen({navigation}: Props) {
  const store = useBeaconStore();
  const [error, setError] = useState<string | null>(null);

  const trySample = async () => {
    const result = await store.loadSampleHousehold();
    setError(result.ok ? null : result.error);
  };

  return (
    <Screen>
      <Gap size={32} />
      <Title>Family Beacon</Title>
      <Body muted>Stay in touch with your household when phones and power are out.</Body>
      <Gap size={8} />
      {POINTS.map(([title, detail]) => (
        <Card key={title}>
          <Body>{title}</Body>
          <Small>{detail}</Small>
        </Card>
      ))}
      <Gap size={8} />
      <Button label="Set up my family circle" onPress={() => navigation.navigate('Circle')} />
      {config.offerSampleHousehold ? (
        <>
          <Button variant="secondary" label="Explore with a sample family" onPress={trySample} />
          <Small>Development only: loads the sample household from /fixtures.</Small>
        </>
      ) : null}
      <InlineError message={error} />
    </Screen>
  );
}
