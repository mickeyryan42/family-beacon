import React from 'react';
import type {BottomTabScreenProps} from '@react-navigation/bottom-tabs';
import type {CompositeScreenProps} from '@react-navigation/native';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {Body, Button, Card, EmptyState, Screen, Small} from '../components/ui';
import type {RootStackParamList, TabParamList} from '../navigation/types';
import {useBeacon} from '../state/BeaconProvider';

type Props = CompositeScreenProps<
  BottomTabScreenProps<TabParamList, 'Rendezvous'>,
  NativeStackScreenProps<RootStackParamList>
>;

const RANK_LABEL = ['Primary', 'First backup', 'Second backup'];

export function RendezvousScreen({navigation}: Props) {
  const {view} = useBeacon();
  const points = view.rendezvous;

  return (
    <Screen>
      <Body muted>
        If you can't reach each other, everyone goes to the first meeting point. If that doesn't
        work, try the next one.
      </Body>
      {points.length === 0 ? (
        <EmptyState
          title="No meeting points yet"
          body="Pick a place everyone knows, like a neighbor's porch or the school gate. Add a backup farther away."
        />
      ) : (
        points.map((p, i) => (
          <Card key={p.id}>
            <Small>{RANK_LABEL[i] ?? `Backup ${i}`}</Small>
            <Body>{p.name}</Body>
            {p.details ? <Small>{p.details}</Small> : null}
            {p.when_to_go ? <Small>When: {p.when_to_go}</Small> : null}
            <Button
              variant="secondary"
              label="Edit"
              onPress={() => navigation.navigate('EditRendezvous', {id: p.id})}
            />
          </Card>
        ))
      )}
      <Button label="Add a meeting point" onPress={() => navigation.navigate('EditRendezvous')} />
    </Screen>
  );
}
