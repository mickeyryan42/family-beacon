/**
 * Always-visible status strip. The point is reassurance: losing signal is the
 * normal case this app is built for, so the banner explains that the app still
 * works rather than warning that something is broken.
 */
import React from 'react';
import {StyleSheet, Text, View} from 'react-native';

import {useBeacon, useConnectivity} from '../state/BeaconProvider';
import {spacing, typography, useTheme} from '../theme/theme';

export function OfflineBanner() {
  const {colors} = useTheme();
  const connectivity = useConnectivity();
  const {transport, nearby} = useBeacon();

  const offline = connectivity === 'no-signal';
  const headline = offline ? 'No signal. Family Beacon still works.' : 'Working locally';
  const detail = offline
    ? 'Updates are saved on this phone and shared when family phones are nearby.'
    : 'Nothing is sent to the internet. Updates go phone to phone.';

  const radio =
    transport.status.state === 'unavailable'
      ? `${transport.label}: not available yet`
      : nearby.length > 0
      ? `${nearby.length} ${nearby.length === 1 ? 'phone' : 'phones'} nearby`
      : transport.kind === 'mock'
      ? 'Mock transport (development)'
      : `${transport.label}: listening`;

  return (
    <View
      accessibilityRole="summary"
      accessibilityLabel={`${headline} ${detail}`}
      testID="offline-banner"
      style={[
        styles.banner,
        {
          backgroundColor: offline ? colors.bannerOffline : colors.bannerOnline,
          borderColor: colors.border,
        },
      ]}>
      <Text style={[typography.small, styles.headline, {color: colors.text}]}>{headline}</Text>
      <Text style={[typography.small, {color: colors.textMuted}]}>{detail}</Text>
      <Text style={[typography.small, {color: colors.textMuted}]}>{radio}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  banner: {
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 2,
  },
  headline: {fontWeight: '700'},
});
