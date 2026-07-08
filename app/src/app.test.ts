import { describe, expect, it } from 'vitest';
import { renderApp } from './app';

describe('W4DJ desktop UI', () => {
  it('renders the compact one-screen layout', () => {
    const root = renderApp({
      sourceDirectory: '/music/in',
      destinationDirectory: '/music/out',
      mode: 'compat',
      losslessFormat: null,
      status: 'idle',
      progressText: 'Ready',
      currentFile: '',
      logExpanded: false,
      logs: [],
    });

    expect(root.querySelector('[data-role="source-picker"]')).not.toBeNull();
    expect(root.querySelector('[data-role="destination-picker"]')).not.toBeNull();
    expect(root.querySelector('[data-role="mode-switch"]')).not.toBeNull();
    expect(root.querySelector('[data-role="status-strip"]')).not.toBeNull();
    expect(root.querySelector('[data-role="log-drawer"]')).not.toBeNull();
    expect(root.textContent).toContain('开始');
  });

  it('keeps logs collapsed until requested', () => {
    const root = renderApp({
      sourceDirectory: '/music/in',
      destinationDirectory: '/music/out',
      mode: 'lossless',
      losslessFormat: 'flac',
      status: 'running',
      progressText: '2 / 8',
      currentFile: 'track.flac',
      logExpanded: false,
      logs: ['Sync started'],
    });

    const drawer = root.querySelector('[data-role="log-drawer"]') as HTMLElement;

    expect(drawer.hidden).toBe(true);
    expect(root.querySelector('[data-role="status-strip"]')?.textContent).toContain('track.flac');
  });
});
