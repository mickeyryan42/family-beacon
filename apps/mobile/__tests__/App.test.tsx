import React from 'react';
import {act, fireEvent, render, screen} from '@testing-library/react-native';

import App from '../App';
import {MemoryRepository} from '../src/data/repository';
import {BeaconStore} from '../src/state/beaconStore';

type Element = ReturnType<typeof screen.getByText>;

/** Press something whose handler updates the store asynchronously. */
async function press(element: Element) {
  await act(async () => {
    fireEvent.press(element);
  });
}

let store: BeaconStore;

async function renderApp() {
  store = new BeaconStore(new MemoryRepository(), Date.now, 'mock');
  render(<App store={store} />);
  await screen.findByText('Family Beacon');
}

afterEach(() => store?.dispose());

describe('App', () => {
  it('boots to onboarding, then opens the sample family', async () => {
    await renderApp();
    await press(screen.getByText('Explore with a sample family'));

    expect(await screen.findByText('How are you?')).toBeTruthy();
    expect(screen.getByText('Family only. Neighbors never see check-ins.')).toBeTruthy();
    expect(screen.getByTestId('offline-banner')).toBeTruthy();
    expect(screen.getAllByText('Jordan').length).toBeGreaterThan(0);
  });

  it('creates a circle, adds a member, and finishes with Done', async () => {
    await renderApp();
    await press(screen.getByText('Set up my family circle'));
    fireEvent.changeText(await screen.findByLabelText('Circle name'), 'The Rivera family');
    fireEvent.changeText(screen.getByLabelText('Your name'), 'Ana');
    await press(screen.getByText('Create circle'));

    fireEvent.changeText(await screen.findByLabelText('Name'), 'Leo');
    await press(screen.getByText('Child'));
    await press(screen.getByText('Add to circle'));
    expect(await screen.findByText('Added Leo to your circle.')).toBeTruthy();
    expect(store.getSnapshot().household?.members.map(m => m.role)).toEqual(['adult', 'child']);

    await press(screen.getByText('Done'));
    expect(await screen.findByText('How are you?')).toBeTruthy();
  });

  it('joins a neighborhood and posts an alert with a clear scope', async () => {
    await renderApp();
    await press(screen.getByText('Explore with a sample family'));
    await press(await screen.findByText('Neighbors'));

    expect(await screen.findByText('What they never see')).toBeTruthy();
    await press(screen.getByText('Simulate joining Elm & Oak'));

    expect(await screen.findByText('Shared with Elm & Oak · 3 people')).toBeTruthy();
    fireEvent.changeText(screen.getByLabelText('What is happening?'), 'Transformer down on Elm');
    await press(screen.getByText('Share with neighborhood'));

    expect(await screen.findByText('Power alert')).toBeTruthy();
    expect(screen.getByText('Transformer down on Elm')).toBeTruthy();
  });
});
