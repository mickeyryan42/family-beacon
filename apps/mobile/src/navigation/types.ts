import type {NavigatorScreenParams} from '@react-navigation/native';

export type TabParamList = {
  CheckIns: undefined;
  Rendezvous: undefined;
  Neighbors: undefined;
  Nearby: undefined;
  Settings: undefined;
};

export type RootStackParamList = {
  Welcome: undefined;
  Circle: undefined;
  Main: NavigatorScreenParams<TabParamList>;
  EditRendezvous: {id?: string} | undefined;
  NeighborhoodPeople: {neighborhoodId: string};
  Conversation: {personId: string};
};
