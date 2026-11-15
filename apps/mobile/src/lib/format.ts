import type {AlertCategory, CheckInState, PersonRole, RemovalReason} from '../sync/protocol';

export const CHECK_IN_LABELS: Record<CheckInState, string> = {
  safe: "I'm safe",
  on_my_way: 'On my way',
  at_rendezvous: 'At the meeting point',
  need_help: 'I need help',
};

export const ROLE_LABELS: Record<PersonRole, string> = {
  adult: 'Adult',
  child: 'Child',
  caregiver: 'Caregiver',
};

export function timeAgo(at: number, now: number = Date.now()): string {
  const minutes = Math.round((now - at) / 60_000);
  if (minutes < 1) {
    return 'just now';
  }
  if (minutes < 60) {
    return `${minutes} min ago`;
  }
  const hours = Math.round(minutes / 60);
  if (hours < 24) {
    return `${hours} hr ago`;
  }
  const days = Math.round(hours / 24);
  return `${days} day${days === 1 ? '' : 's'} ago`;
}

export const ALERT_LABELS: Record<AlertCategory, string> = {
  power: 'Power',
  water: 'Water',
  road: 'Road',
  medical: 'Medical',
  fire: 'Fire',
  other: 'Other',
};

export const REMOVAL_LABELS: Record<RemovalReason, string> = {
  spam: 'Spam',
  abuse: 'Abuse or harassment',
  left_area: 'Moved away',
  other: 'Other',
};

/** One honest line about how sharing works without a network. */
export const DELIVERY_NOTE =
  'Saved on this phone. Reaches people as phones pass each other, often within an hour. No internet needed.';
