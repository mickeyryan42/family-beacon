/**
 * Neighborhood people: who's in, inviting in person, and steward tools.
 *
 * Invite chains (who invited whom) are visible to stewards only.
 */
import React, {useState} from 'react';
import {View} from 'react-native';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {
  Body,
  Button,
  Card,
  Field,
  Heading,
  InlineError,
  Screen,
  Segmented,
  Small,
  ToggleRow,
} from '../components/ui';
import {config} from '../config';
import {REMOVAL_LABELS} from '../lib/format';
import type {RootStackParamList} from '../navigation/types';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';
import type {ActionResult, NeighborhoodSummary} from '../state/beaconStore';
import {INVITES_PER_DAY, SOFT_MEMBER_CAP} from '../sync/neighborhoodView';
import type {MembershipCert, RemovalReason, Uuid} from '../sync/protocol';
import {spacing, useTheme} from '../theme/theme';

type Props = NativeStackScreenProps<RootStackParamList, 'NeighborhoodPeople'>;

const REASONS = Object.keys(REMOVAL_LABELS) as RemovalReason[];

export function NeighborhoodPeopleScreen({navigation, route}: Props) {
  const {neighborhoods} = useBeacon();
  const hood = neighborhoods.find(n => n.info.id === route.params.neighborhoodId);

  if (!hood) {
    return (
      <Screen>
        <Body muted>This neighborhood is no longer on this phone.</Body>
      </Screen>
    );
  }

  return (
    <Screen>
      <Heading>{hood.info.name}</Heading>
      <Small>
        {hood.members.length} people
        {hood.isFounder ? ' · you founded it' : hood.isSteward ? ' · you are a steward' : ''}
      </Small>
      {hood.overSoftCap ? (
        <Card>
          <Body>This neighborhood is getting big</Body>
          <Small>
            Past {SOFT_MEMBER_CAP} people, updates travel more slowly. Consider starting a separate
            neighborhood for part of the area.
          </Small>
        </Card>
      ) : null}
      <Invite hood={hood} />
      <Heading>Members</Heading>
      {hood.members.map(m => (
        <Member
          key={m.member}
          hood={hood}
          cert={m}
          onMessage={() => navigation.navigate('Conversation', {personId: m.member})}
        />
      ))}
    </Screen>
  );
}

function Invite({hood}: {hood: NeighborhoodSummary}) {
  const store = useBeaconStore();
  const {colors} = useTheme();
  const [name, setName] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);

  if (hood.removedMe) {
    return null;
  }
  const invite = async () => {
    const result = await store.inviteNeighbor(hood.info.id, name);
    if (result.ok) {
      setDone(`${name.trim()} joined ${hood.info.name}.`);
      setName('');
      setError(null);
    } else {
      setDone(null);
      setError(result.error);
    }
  };

  return (
    <Card>
      <Body>Invite a neighbor</Body>
      <Small>
        Invites happen in person: they show a code on their phone and you scan it. Nobody can join
        remotely. You can invite {hood.invitesLeftToday} more of {INVITES_PER_DAY} people today.
      </Small>
      {config.offerSampleHousehold ? (
        <>
          <Field
            label="Their name (development: simulates the scan)"
            placeholder="e.g. Priya Shah"
            value={name}
            onChangeText={setName}
            maxLength={80}
          />
          <InlineError message={error} />
          {done ? <Small color={colors.status.safe}>{done}</Small> : null}
          <Button
            label="Simulate inviting a neighbor"
            disabled={!name.trim() || hood.invitesLeftToday === 0}
            onPress={invite}
          />
        </>
      ) : (
        <Small>Code scanning arrives with camera support.</Small>
      )}
    </Card>
  );
}

function Member({
  hood,
  cert,
  onMessage,
}: {
  hood: NeighborhoodSummary;
  cert: MembershipCert;
  onMessage: () => void;
}) {
  const store = useBeaconStore();
  const {device, people, blocked} = useBeacon();
  const [removing, setRemoving] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const me = device?.personId;
  const isMe = cert.member === me;
  const isFounder = cert.member === hood.info.founder;
  const isSteward = hood.stewards.includes(cert.member);
  const isBlocked = blocked.includes(cert.member);
  const badges = [isMe && 'you', isFounder && 'founder', !isFounder && isSteward && 'steward']
    .filter(Boolean)
    .join(' · ');

  const chain = hood.isSteward ? store.chainOf(hood.info.id, cert.member) : [];
  const show = (result: ActionResult, ok: string) => setNotice(result.ok ? ok : result.error);

  return (
    <Card>
      <Body>
        {cert.display_name}
        {badges ? ` (${badges})` : ''}
      </Body>
      {hood.isSteward && chain.length > 1 ? (
        <Small>
          Invited by{' '}
          {chain
            .slice(1)
            .map(p => people[p] ?? 'someone')
            .join(', invited by ')}
        </Small>
      ) : null}
      {!isMe ? (
        <View style={{flexDirection: 'row', flexWrap: 'wrap', gap: spacing.sm}}>
          <View style={{flexGrow: 1, minWidth: '30%'}}>
            <Button compact variant="secondary" label="Message" onPress={onMessage} />
          </View>
          <View style={{flexGrow: 1, minWidth: '30%'}}>
            <Button
              compact
              variant="secondary"
              label="Are you OK?"
              onPress={async () =>
                show(
                  await store.askAreYouOk(cert.member),
                  'Asked. You will see their answer in Messages.',
                )
              }
            />
          </View>
          <View style={{flexGrow: 1, minWidth: '30%'}}>
            <Button
              compact
              variant="secondary"
              label={isBlocked ? 'Unblock' : 'Block'}
              onPress={() => (isBlocked ? store.unblock(cert.member) : store.block(cert.member))}
            />
          </View>
        </View>
      ) : null}
      {hood.isFounder && !isSteward && !isMe ? (
        <Button
          variant="secondary"
          label="Make steward"
          onPress={async () =>
            show(
              await store.addSteward(hood.info.id, cert.member),
              `${cert.display_name} can now remove people.`,
            )
          }
        />
      ) : null}
      {hood.isSteward && !isMe && !isFounder ? (
        removing ? (
          <RemovePanel hood={hood} cert={cert} onClose={() => setRemoving(false)} />
        ) : (
          <Button
            variant="danger"
            label="Remove from neighborhood"
            onPress={() => setRemoving(true)}
          />
        )
      ) : null}
      {notice ? <Small>{notice}</Small> : null}
    </Card>
  );
}

function RemovePanel({
  hood,
  cert,
  onClose,
}: {
  hood: NeighborhoodSummary;
  cert: MembershipCert;
  onClose: () => void;
}) {
  const store = useBeaconStore();
  const [reason, setReason] = useState<RemovalReason>('spam');
  // Recommended default: keep the people they invited.
  const [cascade, setCascade] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const invited = store.invitedCount(hood.info.id, cert.member);

  const confirm = async () => {
    const result = await store.removeMember(hood.info.id, cert.member as Uuid, reason, cascade);
    if (result.ok) {
      onClose();
    } else {
      setError(result.error);
    }
  };

  return (
    <View style={{gap: spacing.sm}}>
      <Small>Why are you removing {cert.display_name}?</Small>
      <Segmented
        options={REASONS.map(r => ({value: r, label: REMOVAL_LABELS[r]}))}
        value={reason}
        onChange={setReason}
      />
      {invited > 0 ? (
        <ToggleRow
          label={`Also remove the ${invited} ${invited === 1 ? 'person' : 'people'} they invited`}
          detail="Usually not needed. Turn on if they brought in people who shouldn't be here."
          value={cascade}
          onValueChange={setCascade}
        />
      ) : null}
      <Small>
        Everyone else gets a new neighborhood key, so {cert.display_name} can't read anything posted
        from now on.
      </Small>
      <InlineError message={error} />
      <Button variant="danger" label={`Remove ${cert.display_name}`} onPress={confirm} />
      <Button variant="secondary" label="Cancel" onPress={onClose} />
    </View>
  );
}
