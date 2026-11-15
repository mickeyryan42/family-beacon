import React from 'react';
import {StatusBar} from 'react-native';
import {SafeAreaProvider, SafeAreaView} from 'react-native-safe-area-context';

import {ErrorBoundary} from './components/ErrorBoundary';
import {RootNavigator} from './navigation/RootNavigator';
import {BeaconProvider} from './state/BeaconProvider';
import type {BeaconStore} from './state/beaconStore';
import {ThemeProvider, useTheme} from './theme/theme';

function Shell() {
  const {colors, isDark} = useTheme();
  return (
    <SafeAreaView edges={['top']} style={{flex: 1, backgroundColor: colors.background}}>
      <StatusBar
        barStyle={isDark ? 'light-content' : 'dark-content'}
        backgroundColor={colors.background}
      />
      <RootNavigator />
    </SafeAreaView>
  );
}

export default function App({store}: {store?: BeaconStore}) {
  return (
    <ErrorBoundary>
      <SafeAreaProvider>
        <ThemeProvider>
          <BeaconProvider store={store}>
            <Shell />
          </BeaconProvider>
        </ThemeProvider>
      </SafeAreaProvider>
    </ErrorBoundary>
  );
}
