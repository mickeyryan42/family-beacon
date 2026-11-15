import React from 'react';
import {ScrollView, StyleSheet, Text} from 'react-native';

import {describeError, logger} from '../lib/logger';

const log = logger('ui');

interface State {
  error: Error | null;
}

/**
 * Last-resort catch for render errors. In development it shows the message and
 * stack on screen (handy on a device away from Metro); in release it shows a
 * plain apology and a way back. Uses fixed colors so it works even if the
 * theme provider is what failed.
 */
export class ErrorBoundary extends React.Component<{children: React.ReactNode}, State> {
  state: State = {error: null};

  static getDerivedStateFromError(error: Error): State {
    return {error};
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    log.error(`render error: ${describeError(error)}`, info.componentStack);
  }

  render() {
    const {error} = this.state;
    if (!error) {
      return this.props.children;
    }
    return (
      <ScrollView style={styles.root} contentContainerStyle={styles.content}>
        <Text style={styles.title}>Something went wrong</Text>
        <Text style={styles.body}>
          Your family's updates are still saved on this phone. Please close and reopen the app.
        </Text>
        {__DEV__ ? (
          <>
            <Text style={styles.devTitle}>{error.message}</Text>
            <Text style={styles.stack}>{error.stack}</Text>
            <Text style={styles.retry} onPress={() => this.setState({error: null})}>
              Try again
            </Text>
          </>
        ) : null}
      </ScrollView>
    );
  }
}

const styles = StyleSheet.create({
  root: {flex: 1, backgroundColor: '#0F1418'},
  content: {padding: 24, gap: 12, paddingTop: 80},
  title: {color: '#ECF1F4', fontSize: 24, fontWeight: '700'},
  body: {color: '#9AA8B3', fontSize: 17},
  devTitle: {color: '#EE7B6B', fontSize: 16, fontWeight: '600', marginTop: 16},
  stack: {color: '#9AA8B3', fontSize: 12, fontFamily: 'Courier'},
  retry: {color: '#F2B84B', fontSize: 18, fontWeight: '600', paddingVertical: 16},
});
