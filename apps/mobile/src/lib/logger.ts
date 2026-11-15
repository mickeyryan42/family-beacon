/**
 * Small tagged logger.
 *
 * In development everything at or above `config.logLevel` goes to the Metro
 * console with a tag, and the most recent entries are kept in memory so the
 * Settings screen can show them on-device (useful when testing away from a
 * laptop). Release builds only keep warnings and errors, and nothing is ever
 * sent off the device.
 */
import {config, type LogLevel} from '../config';

const ORDER: Record<LogLevel, number> = {debug: 10, info: 20, warn: 30, error: 40};
const MAX_ENTRIES = 200;

export interface LogEntry {
  at: number;
  level: LogLevel;
  tag: string;
  message: string;
  detail?: unknown;
}

const entries: LogEntry[] = [];

function write(level: LogLevel, tag: string, message: string, detail?: unknown) {
  if (ORDER[level] < ORDER[config.logLevel]) {
    return;
  }
  entries.push({at: Date.now(), level, tag, message, detail});
  if (entries.length > MAX_ENTRIES) {
    entries.splice(0, entries.length - MAX_ENTRIES);
  }
  if (!__DEV__ && ORDER[level] < ORDER.warn) {
    return;
  }
  const line = `[${tag}] ${message}`;
  const args = detail === undefined ? [line] : [line, detail];
  switch (level) {
    case 'error':
      console.error(...args);
      break;
    case 'warn':
      console.warn(...args);
      break;
    default:
      console.log(...args);
  }
}

export function logger(tag: string) {
  return {
    debug: (message: string, detail?: unknown) => write('debug', tag, message, detail),
    info: (message: string, detail?: unknown) => write('info', tag, message, detail),
    warn: (message: string, detail?: unknown) => write('warn', tag, message, detail),
    error: (message: string, detail?: unknown) => write('error', tag, message, detail),
  };
}

export function recentLogs(): readonly LogEntry[] {
  return entries;
}

export function describeError(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }
  return typeof error === 'string' ? error : 'Unknown error';
}
