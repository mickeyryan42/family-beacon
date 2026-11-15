/**
 * Small set of building blocks every screen uses. Big tap targets, plain
 * language, no animation.
 */
import React from 'react';
import {
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  TextInput,
  View,
  type TextInputProps,
  type ViewStyle,
} from 'react-native';

import {radius, spacing, TAP_TARGET, typography, useTheme} from '../theme/theme';

export function Screen({
  children,
  scroll = true,
  footer,
}: {
  children: React.ReactNode;
  scroll?: boolean;
  /** Pinned below the scrolling content, e.g. a "Done" button that must always be reachable. */
  footer?: React.ReactNode;
}) {
  const {colors} = useTheme();
  const style = {flex: 1, backgroundColor: colors.background};
  const body = !scroll ? (
    <View style={[style, styles.screenPad]}>{children}</View>
  ) : (
    <ScrollView
      style={style}
      contentContainerStyle={styles.screenPad}
      keyboardShouldPersistTaps="handled">
      {children}
    </ScrollView>
  );
  if (!footer) {
    return body;
  }
  return (
    <View style={style}>
      {body}
      <View
        style={[
          styles.footer,
          {borderTopColor: colors.border, backgroundColor: colors.background},
        ]}>
        {footer}
      </View>
    </View>
  );
}

export function Title({children}: {children: React.ReactNode}) {
  const {colors} = useTheme();
  return <Text style={[typography.title, {color: colors.text}]}>{children}</Text>;
}

export function Heading({children}: {children: React.ReactNode}) {
  const {colors} = useTheme();
  return (
    <Text
      accessibilityRole="header"
      style={[typography.heading, styles.heading, {color: colors.text}]}>
      {children}
    </Text>
  );
}

export function Body({children, muted}: {children: React.ReactNode; muted?: boolean}) {
  const {colors} = useTheme();
  return (
    <Text style={[typography.body, {color: muted ? colors.textMuted : colors.text}]}>
      {children}
    </Text>
  );
}

export function Small({children, color}: {children: React.ReactNode; color?: string}) {
  const {colors} = useTheme();
  return <Text style={[typography.small, {color: color ?? colors.textMuted}]}>{children}</Text>;
}

export function Card({children, style}: {children: React.ReactNode; style?: ViewStyle}) {
  const {colors} = useTheme();
  return (
    <View
      style={[styles.card, {backgroundColor: colors.surface, borderColor: colors.border}, style]}>
      {children}
    </View>
  );
}

type ButtonVariant = 'primary' | 'secondary' | 'danger';

export function Button({
  label,
  onPress,
  variant = 'primary',
  disabled,
  accessibilityHint,
  compact,
}: {
  label: string;
  onPress: () => void;
  variant?: ButtonVariant;
  disabled?: boolean;
  accessibilityHint?: string;
  /** Less side padding, for buttons laid out several to a row. */
  compact?: boolean;
}) {
  const {colors} = useTheme();
  const background =
    variant === 'primary'
      ? colors.accent
      : variant === 'danger'
      ? colors.danger
      : colors.surfaceRaised;
  const foreground = variant === 'secondary' ? colors.text : colors.onAccent;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityHint={accessibilityHint}
      accessibilityState={{disabled}}
      disabled={disabled}
      onPress={onPress}
      style={({pressed}) => [
        styles.button,
        {
          backgroundColor: background,
          borderColor: variant === 'secondary' ? colors.border : background,
          opacity: disabled ? 0.5 : pressed ? 0.85 : 1,
          paddingHorizontal: compact ? spacing.sm : spacing.lg,
        },
      ]}>
      <Text
        numberOfLines={1}
        adjustsFontSizeToFit
        style={[styles.buttonLabel, {color: foreground}]}>
        {label}
      </Text>
    </Pressable>
  );
}

/** A large, full-width choice used for one-tap check-ins and option lists. */
export function ChoiceRow({
  label,
  detail,
  dotColor,
  selected,
  onPress,
}: {
  label: string;
  detail?: string;
  dotColor?: string;
  selected?: boolean;
  onPress: () => void;
}) {
  const {colors} = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{selected}}
      onPress={onPress}
      style={({pressed}) => [
        styles.choice,
        {
          backgroundColor: selected ? colors.surfaceRaised : colors.surface,
          borderColor: selected ? colors.accent : colors.border,
          opacity: pressed ? 0.85 : 1,
        },
      ]}>
      {dotColor ? <StatusDot color={dotColor} /> : null}
      <View style={styles.flex}>
        <Text style={[typography.body, {color: colors.text, fontWeight: '600'}]}>{label}</Text>
        {detail ? <Small>{detail}</Small> : null}
      </View>
    </Pressable>
  );
}

export function StatusDot({color, size = 14}: {color: string; size?: number}) {
  return (
    <View
      style={{width: size, height: size, borderRadius: size / 2, backgroundColor: color}}
      accessibilityElementsHidden
      importantForAccessibility="no"
    />
  );
}

export function Field({label, ...props}: TextInputProps & {label: string}) {
  const {colors} = useTheme();
  return (
    <View style={styles.field}>
      <Small>{label}</Small>
      <TextInput
        accessibilityLabel={label}
        placeholderTextColor={colors.textMuted}
        style={[
          styles.input,
          typography.body,
          {color: colors.text, backgroundColor: colors.surface, borderColor: colors.border},
        ]}
        {...props}
      />
    </View>
  );
}

export function ToggleRow({
  label,
  detail,
  value,
  onValueChange,
}: {
  label: string;
  detail?: string;
  value: boolean;
  onValueChange: (v: boolean) => void;
}) {
  const {colors} = useTheme();
  return (
    <View style={styles.toggleRow}>
      <View style={styles.flex}>
        <Body>{label}</Body>
        {detail ? <Small>{detail}</Small> : null}
      </View>
      <Switch
        accessibilityLabel={label}
        value={value}
        onValueChange={onValueChange}
        trackColor={{true: colors.accent, false: colors.border}}
      />
    </View>
  );
}

export function EmptyState({
  title,
  body,
  action,
}: {
  title: string;
  body: string;
  action?: React.ReactNode;
}) {
  const {colors} = useTheme();
  return (
    <View style={[styles.empty, {borderColor: colors.border}]}>
      <Text style={[typography.heading, {color: colors.text, textAlign: 'center'}]}>{title}</Text>
      <Text style={[typography.body, {color: colors.textMuted, textAlign: 'center'}]}>{body}</Text>
      {action}
    </View>
  );
}

export function InlineError({message}: {message: string | null}) {
  const {colors} = useTheme();
  if (!message) {
    return null;
  }
  return (
    <Text accessibilityLiveRegion="polite" style={[typography.small, {color: colors.danger}]}>
      {message}
    </Text>
  );
}

export function Gap({size = spacing.md}: {size?: number}) {
  return <View style={{height: size}} />;
}

const styles = StyleSheet.create({
  screenPad: {padding: spacing.md, paddingBottom: spacing.xl, gap: spacing.md},
  footer: {padding: spacing.md, borderTopWidth: StyleSheet.hairlineWidth},
  heading: {marginTop: spacing.sm},
  card: {borderWidth: 1, borderRadius: radius.lg, padding: spacing.md, gap: spacing.sm},
  button: {
    minHeight: TAP_TARGET,
    borderRadius: radius.md,
    borderWidth: 1,
    paddingHorizontal: spacing.lg,
    alignItems: 'center',
    justifyContent: 'center',
  },
  buttonLabel: {fontSize: 18, fontWeight: '600'},
  choice: {
    minHeight: TAP_TARGET + 8,
    borderRadius: radius.md,
    borderWidth: 2,
    paddingHorizontal: spacing.md,
    paddingVertical: spacing.sm,
    flexDirection: 'row',
    alignItems: 'center',
    gap: spacing.md,
  },
  flex: {flex: 1},
  field: {gap: spacing.xs},
  input: {
    minHeight: TAP_TARGET,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingHorizontal: spacing.md,
  },
  toggleRow: {minHeight: TAP_TARGET, flexDirection: 'row', alignItems: 'center', gap: spacing.md},
  empty: {
    borderWidth: 1,
    borderStyle: 'dashed',
    borderRadius: radius.lg,
    padding: spacing.lg,
    gap: spacing.sm,
    alignItems: 'center',
  },
});

export type ScopeKind = 'household' | 'neighborhood' | 'direct';

/**
 * States who will see what's being written. Shown on every compose area, so
 * nobody shares family details with the neighborhood by accident.
 */
export function ScopeBadge({scope, label}: {scope: ScopeKind; label: string}) {
  const {colors} = useTheme();
  const color =
    scope === 'household'
      ? colors.status.safe
      : scope === 'neighborhood'
      ? colors.status.on_my_way
      : colors.status.at_rendezvous;
  return (
    <View
      accessibilityLabel={`Visible to: ${label}`}
      style={{flexDirection: 'row', alignItems: 'center', gap: spacing.sm}}>
      <StatusDot color={color} size={10} />
      <Text style={[typography.small, {color, fontWeight: '700'}]}>{label}</Text>
    </View>
  );
}

/** A compact row of mutually exclusive options. */
export function Segmented<T extends string>({
  options,
  value,
  onChange,
}: {
  options: {value: T; label: string}[];
  value: T;
  onChange: (v: T) => void;
}) {
  return (
    <View style={{flexDirection: 'row', flexWrap: 'wrap', gap: spacing.sm}}>
      {options.map(o => (
        <View key={o.value} style={{flexGrow: 1, minWidth: '22%'}}>
          <Button
            compact
            variant={o.value === value ? 'primary' : 'secondary'}
            label={o.label}
            onPress={() => onChange(o.value)}
          />
        </View>
      ))}
    </View>
  );
}
