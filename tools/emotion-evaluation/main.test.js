// @vitest-environment jsdom

import { beforeEach, describe, expect, it, vi } from 'vitest';

const manifest = {
  schemaVersion: 1,
  sessionId: 'ui-session',
  seed: 7,
  sampleSize: 1,
  clipPolicy: 'peak-energy-10s-with-drop-preference',
  tracks: [{
    trackId: 'track-1',
    title: 'Test song',
    artist: 'Test artist',
    album: 'Test album',
    relativePath: 'Album/test.mp3',
    durationSeconds: 12,
    clipStartSeconds: 1,
    clipDurationSeconds: 10,
    clipSelection: 'peakEnergy',
    legacyMood: { status: 'completed', labels: [{ label: 'happy' }] },
    emomusic: { status: 'completed', valence: 7, arousal: 6 },
    muse: { status: 'completed', valence: 6, arousal: 5 },
    mirex: { status: 'completed', labels: [{ label: 'joy' }] },
  }],
};

function fixture() {
  document.body.innerHTML = `
    <input id="manifest-input"><input id="audio-input">
    <p id="setup-status"></p><button id="start-button"></button><button id="resume-button"></button>
    <section id="setup"></section><section id="evaluation" hidden>
      <span id="progress-text"></span><button id="previous-button"></button><button id="pause-button"></button>
      <h2 id="track-title"></h2><p id="track-artist"></p><p id="track-path"></p>
      <audio id="audio-player"></audio><p id="audio-status"></p>
      <div id="human-stage"><div id="human-options"></div><button id="human-submit"></button></div>
      <div id="model-stage" hidden><div id="model-cards"></div><button id="tie-button"></button>
        <button id="none-button"></button><button id="model-submit"></button></div>
    </section>
    <section id="summary" hidden><pre id="summary-output"></pre><button id="edit-last-button"></button>
      <button id="export-json"></button><button id="export-csv"></button></section>`;
}

describe('emotion evaluation page flow', () => {
  beforeEach(() => {
    fixture();
    localStorage.clear();
    vi.resetModules();
    vi.stubGlobal('URL', {
      ...URL,
      createObjectURL: vi.fn(() => 'blob:test'),
      revokeObjectURL: vi.fn(),
    });
  });

  it('requires a human label, then records an anonymous model comparison', async () => {
    await import('./main.js');
    const manifestInput = document.querySelector('#manifest-input');
    const audioInput = document.querySelector('#audio-input');
    Object.defineProperty(manifestInput, 'files', {
      configurable: true,
      value: [{ text: async () => JSON.stringify(manifest) }],
    });
    Object.defineProperty(audioInput, 'files', {
      configurable: true,
      value: [{ name: 'test.mp3', webkitRelativePath: 'output/Album/test.mp3' }],
    });
    manifestInput.dispatchEvent(new Event('change'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    audioInput.dispatchEvent(new Event('change'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.querySelector('#start-button').disabled).toBe(false);
    document.querySelector('#start-button').click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.querySelector('#evaluation').hidden).toBe(false);
    expect(document.querySelector('#human-submit').disabled).toBe(true);

    document.querySelector('#human-options button').click();
    expect(document.querySelector('#human-submit').disabled).toBe(false);
    document.querySelector('#human-submit').click();
    expect(document.querySelector('#model-stage').hidden).toBe(false);
    const cards = [...document.querySelectorAll('#model-cards button:not(:disabled)')];
    cards[0].click();
    cards[1].click();
    expect(document.querySelector('#model-submit').disabled).toBe(false);
    document.querySelector('#model-submit').click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.querySelector('#summary').hidden).toBe(false);
    expect(JSON.parse(document.querySelector('#summary-output').textContent).validSamples).toBe(1);
  });

  it('restores a session that was closed after the human-label stage', async () => {
    const session = {
      schemaVersion: 1,
      sessionId: manifest.sessionId,
      manifest,
      trackOrder: ['track-1'],
      cards: { 'track-1': ['muse', 'legacyMood', 'mirex', 'emomusic'] },
      cursor: 0,
      phase: 'models',
      pendingHumanLabel: 'bright',
      answers: {},
      paused: false,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    };
    localStorage.setItem(`w4dj-emotion-session:${manifest.sessionId}`, JSON.stringify(session));
    await import('./main.js');
    const manifestInput = document.querySelector('#manifest-input');
    Object.defineProperty(manifestInput, 'files', {
      configurable: true,
      value: [{ text: async () => JSON.stringify(manifest) }],
    });
    manifestInput.dispatchEvent(new Event('change'));
    await new Promise((resolve) => setTimeout(resolve, 0));
    document.querySelector('#resume-button').click();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(document.querySelector('#evaluation').hidden).toBe(false);
    expect(document.querySelector('#human-stage').hidden).toBe(true);
    expect(document.querySelectorAll('#model-cards button')).toHaveLength(4);
  });
});
