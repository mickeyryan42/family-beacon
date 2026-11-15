/**
 * App configuration.
 *
 * Deliberately plain constants rather than a .env loader: there are no
 * secrets or server URLs to configure (the app has no cloud dependency), and
 * avoiding a native config module keeps setup friction-free. Change values
 * here, or flip them at runtime from Settings > Developer.
 */
import type {TransportKind} from './sync/transport';

export type LogLevel = 'debug' | 'info' | 'warn' | 'error';

export const config = {
  /** Transport used at launch. Real radios are not built yet, so dev uses the mock. */
  defaultTransport: (__DEV__ ? 'mock' : 'ble') as TransportKind,

  /** How often a simulated family phone comes into range (mock transport). */
  mockExchangeIntervalMs: 20_000,

  /**
   * How often open screens are re-derived, so expired check-ins disappear and
   * "5 min ago" stays true even when nothing new arrives.
   */
  viewRefreshIntervalMs: 30_000,

  /** Load the sample household from /fixtures on first launch in development. */
  offerSampleHousehold: __DEV__,

  logLevel: (__DEV__ ? 'debug' : 'warn') as LogLevel,
};
