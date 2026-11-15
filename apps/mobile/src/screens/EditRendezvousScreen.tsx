import React, {useState} from 'react';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {Button, Field, InlineError, Screen, Small} from '../components/ui';
import type {RootStackParamList} from '../navigation/types';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';

type Props = NativeStackScreenProps<RootStackParamList, 'EditRendezvous'>;

export function EditRendezvousScreen({navigation, route}: Props) {
  const store = useBeaconStore();
  const {view} = useBeacon();
  const existing = view.rendezvous.find(p => p.id === route.params?.id);

  const [name, setName] = useState(existing?.name ?? '');
  const [details, setDetails] = useState(existing?.details ?? '');
  const [whenToGo, setWhenToGo] = useState(existing?.when_to_go ?? '');
  const [error, setError] = useState<string | null>(null);

  const save = async () => {
    const result = await store.saveRendezvous({
      id: existing?.id,
      rank: existing?.rank,
      name,
      details,
      when_to_go: whenToGo,
      location: existing?.location,
    });
    if (result.ok) {
      navigation.goBack();
    } else {
      setError(result.error);
    }
  };

  const remove = async () => {
    if (!existing) {
      return;
    }
    const result = await store.removeRendezvous(existing.id);
    if (result.ok) {
      navigation.goBack();
    } else {
      setError(result.error);
    }
  };

  return (
    <Screen>
      <Field
        label="Name"
        placeholder="e.g. Front porch at home"
        value={name}
        onChangeText={setName}
        maxLength={80}
      />
      <Field
        label="How to find it"
        placeholder="e.g. Side gate, by the bike racks"
        value={details}
        onChangeText={setDetails}
        multiline
        maxLength={280}
      />
      <Field
        label="When to go there"
        placeholder="e.g. If we can't reach each other for an hour"
        value={whenToGo}
        onChangeText={setWhenToGo}
        multiline
        maxLength={280}
      />
      <Small>
        Map pins are coming later. Plain directions work for everyone, even without maps.
      </Small>
      <InlineError message={error} />
      <Button label="Save meeting point" onPress={save} />
      {existing ? <Button variant="danger" label="Remove" onPress={remove} /> : null}
    </Screen>
  );
}
