/**
 * Private messages with one person, plus one-tap "Are you OK?" check-ins.
 * Sealed so only that person can read it; neighbors' phones only carry it.
 */
import React, {useEffect, useState} from 'react';
import {View} from 'react-native';
import type {NativeStackScreenProps} from '@react-navigation/native-stack';

import {Body, Button, Card, Field, InlineError, Screen, ScopeBadge, Small} from '../components/ui';
import {CHECK_IN_LABELS, DELIVERY_NOTE, timeAgo} from '../lib/format';
import type {RootStackParamList} from '../navigation/types';
import {useBeacon, useBeaconStore} from '../state/BeaconProvider';
import type {ActionResult} from '../state/beaconStore';
import type {DirectMessage} from '../sync/neighborhoodView';
import {radius, spacing, useTheme} from '../theme/theme';

type Props = NativeStackScreenProps<RootStackParamList, 'Conversation'>;

function describe(m: DirectMessage): string {
  switch (m.kind) {
    case 'message':
      return m.text;
    case 'wellness_request':
      return m.text ?? 'Are you OK?';
    case 'wellness_reply':
      return `${CHECK_IN_LABELS[m.state]}${m.text ? `. ${m.text}` : ''}`;
  }
}

export function ConversationScreen({navigation, route}: Props) {
  const store = useBeaconStore();
  const {inbox, device, people, directLeftToday} = useBeacon();
  const {colors} = useTheme();
  const [text, setText] = useState('');
  const [error, setError] = useState<string | null>(null);

  const other = route.params.personId;
  const me = device?.personId;
  const name = people[other] ?? 'Neighbor';
  const thread = inbox.filter(
    m => (m.from === me && m.to === other) || (m.from === other && m.to === me),
  );

  useEffect(() => navigation.setOptions({title: name}), [navigation, name]);

  // Offer one-tap answers if their latest "Are you OK?" hasn't been answered.
  const lastRequest = [...thread]
    .reverse()
    .find(m => m.kind === 'wellness_request' && m.from === other);
  const answered =
    lastRequest &&
    thread.some(
      m => m.kind === 'wellness_reply' && m.from === me && m.createdAt >= lastRequest.createdAt,
    );

  const run = async (action: Promise<ActionResult>, clear = false) => {
    const result = await action;
    setError(result.ok ? null : result.error);
    if (result.ok && clear) {
      setText('');
    }
  };

  return (
    <Screen>
      <ScopeBadge scope="direct" label={`Private to ${name}`} />
      {thread.length === 0 ? (
        <Body muted>No messages yet. Only {name} can read what you send here.</Body>
      ) : (
        thread.map(m => {
          const mine = m.from === me;
          return (
            <View
              key={m.envelopeId}
              style={{
                alignSelf: mine ? 'flex-end' : 'flex-start',
                maxWidth: '85%',
                padding: spacing.md,
                borderRadius: radius.md,
                backgroundColor: mine ? colors.surfaceRaised : colors.surface,
                borderWidth: 1,
                borderColor:
                  m.kind === 'wellness_reply' && m.state === 'need_help'
                    ? colors.status.need_help
                    : colors.border,
                gap: spacing.xs,
              }}>
              <Body>{describe(m)}</Body>
              <Small>
                {mine ? 'You' : name} · {timeAgo(m.createdAt)}
              </Small>
            </View>
          );
        })
      )}

      {lastRequest && !answered ? (
        <Card>
          <Body>{name} asked if you're OK</Body>
          <View style={{flexDirection: 'row', gap: spacing.sm}}>
            <View style={{flex: 1}}>
              <Button
                compact
                label="I'm safe"
                onPress={() => run(store.replyWellness(other, 'safe'))}
              />
            </View>
            <View style={{flex: 1}}>
              <Button
                compact
                variant="danger"
                label="I need help"
                onPress={() => run(store.replyWellness(other, 'need_help'))}
              />
            </View>
          </View>
        </Card>
      ) : null}

      <Field
        label="Message"
        placeholder="Short and clear works best"
        value={text}
        onChangeText={setText}
        maxLength={280}
        multiline
      />
      <InlineError message={error} />
      <Button
        label="Send"
        disabled={!text.trim()}
        onPress={() => run(store.sendMessage(other, text), true)}
      />
      <Button
        variant="secondary"
        label="Ask: Are you OK?"
        onPress={() => run(store.askAreYouOk(other))}
      />
      <Small>
        {DELIVERY_NOTE} {directLeftToday} private messages left today.
      </Small>
    </Screen>
  );
}
