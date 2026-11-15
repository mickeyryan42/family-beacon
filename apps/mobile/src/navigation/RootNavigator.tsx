import React from 'react';
import {ActivityIndicator, View} from 'react-native';
import {DarkTheme, DefaultTheme, NavigationContainer} from '@react-navigation/native';
import {createBottomTabNavigator} from '@react-navigation/bottom-tabs';
import {createNativeStackNavigator} from '@react-navigation/native-stack';

import {OfflineBanner} from '../components/OfflineBanner';
import {StatusDot} from '../components/ui';
import {CheckInsScreen} from '../screens/CheckInsScreen';
import {CircleScreen} from '../screens/CircleScreen';
import {EditRendezvousScreen} from '../screens/EditRendezvousScreen';
import {ConversationScreen} from '../screens/ConversationScreen';
import {NearbyScreen} from '../screens/NearbyScreen';
import {NeighborhoodPeopleScreen} from '../screens/NeighborhoodPeopleScreen';
import {NeighborhoodScreen} from '../screens/NeighborhoodScreen';
import {RendezvousScreen} from '../screens/RendezvousScreen';
import {SettingsScreen} from '../screens/SettingsScreen';
import {WelcomeScreen} from '../screens/WelcomeScreen';
import {useBeacon} from '../state/BeaconProvider';
import {useTheme} from '../theme/theme';
import type {RootStackParamList, TabParamList} from './types';

const Stack = createNativeStackNavigator<RootStackParamList>();
const Tabs = createBottomTabNavigator<TabParamList>();

/** Deliberately plain: a dot that grows on the active tab. No icon font to link. */
function TabIcon({color, focused}: {color: string; focused: boolean}) {
  return <StatusDot color={color} size={focused ? 12 : 8} />;
}

function MainTabs() {
  const {colors} = useTheme();
  return (
    <Tabs.Navigator
      screenOptions={{
        // Minimal motion: no tab transition animation.
        animation: 'none',
        tabBarActiveTintColor: colors.accent,
        tabBarInactiveTintColor: colors.textMuted,
        tabBarStyle: {backgroundColor: colors.surface, borderTopColor: colors.border, height: 72},
        tabBarItemStyle: {paddingVertical: 8},
        tabBarLabelStyle: {fontSize: 13, fontWeight: '600'},
        tabBarIcon: TabIcon,
        headerStyle: {backgroundColor: colors.background},
        headerTintColor: colors.text,
        headerShadowVisible: false,
      }}>
      <Tabs.Screen name="CheckIns" component={CheckInsScreen} options={{title: 'Check-ins'}} />
      <Tabs.Screen
        name="Rendezvous"
        component={RendezvousScreen}
        options={{title: 'Plans', headerTitle: 'Meet-up plans'}}
      />
      <Tabs.Screen
        name="Neighbors"
        component={NeighborhoodScreen}
        options={{title: 'Neighbors', headerTitle: 'Neighborhood'}}
      />
      <Tabs.Screen name="Nearby" component={NearbyScreen} options={{title: 'Nearby'}} />
      <Tabs.Screen name="Settings" component={SettingsScreen} options={{title: 'Settings'}} />
    </Tabs.Navigator>
  );
}

export function RootNavigator() {
  const {colors, isDark} = useTheme();
  const {phase} = useBeacon();

  const base = isDark ? DarkTheme : DefaultTheme;
  const navTheme = {
    ...base,
    colors: {
      ...base.colors,
      background: colors.background,
      card: colors.surface,
      text: colors.text,
      border: colors.border,
      primary: colors.accent,
    },
  };

  if (phase === 'loading') {
    return (
      <View
        style={{
          flex: 1,
          alignItems: 'center',
          justifyContent: 'center',
          backgroundColor: colors.background,
        }}>
        <ActivityIndicator color={colors.accent} size="large" />
      </View>
    );
  }

  return (
    <NavigationContainer theme={navTheme}>
      {phase === 'ready' ? <OfflineBanner /> : null}
      <Stack.Navigator
        screenOptions={{
          animation: 'fade',
          headerStyle: {backgroundColor: colors.background},
          headerTintColor: colors.text,
          headerShadowVisible: false,
          contentStyle: {backgroundColor: colors.background},
        }}>
        {phase === 'onboarding' ? (
          <>
            <Stack.Screen name="Welcome" component={WelcomeScreen} options={{headerShown: false}} />
            <Stack.Screen
              name="Circle"
              component={CircleScreen}
              options={{title: 'Your trusted circle'}}
            />
          </>
        ) : (
          <>
            <Stack.Screen name="Main" component={MainTabs} options={{headerShown: false}} />
            <Stack.Screen
              name="Circle"
              component={CircleScreen}
              options={{title: 'Trusted circle'}}
            />
            <Stack.Screen
              name="EditRendezvous"
              component={EditRendezvousScreen}
              options={{title: 'Meeting point', presentation: 'modal'}}
            />
            <Stack.Screen
              name="NeighborhoodPeople"
              component={NeighborhoodPeopleScreen}
              options={{title: 'People'}}
            />
            <Stack.Screen
              name="Conversation"
              component={ConversationScreen}
              options={{title: 'Private message'}}
            />
          </>
        )}
      </Stack.Navigator>
    </NavigationContainer>
  );
}
