import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn());
const analyzeMock = vi.hoisted(() => vi.fn());
const workerConstructor = vi.hoisted(() => vi.fn());

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }));
vi.mock('./analysis', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./analysis')>();
  return {
    ...actual,
    ESSENTIA_MODEL_IDS: ['musicnn_embedding'],
    analyzeAudioFile: analyzeMock,
  };
});
vi.mock('./analysis-worker-client', async (importOriginal) => {
  const actual = await importOriginal<typeof import('./analysis-worker-client')>();
  return {
    ...actual,
    AnalysisWorkerClient: workerConstructor,
  };
});

import { runLibraryAnalysis } from './library-analysis-runner';

describe('library analysis runner fatal runtime handling', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockImplementation(async (command: string) => {
      switch (command) {
        case 'load_track_analyses': return [];
        case 'ensure_essentia_models': return {};
        case 'load_essentia_model':
          return {
            id: 'musicnn_embedding',
            modelJson: JSON.stringify({ modelTopology: {}, weightsManifest: [] }),
            weightData: [1],
            classes: [],
            kind: 'embedding',
            version: 'test',
          };
        case 'read_audio_file': return [0, 1, 2];
        case 'read_audio_metadata': return { title: 'Long', artist: '', album: '' };
        case 'get_audio_file_fingerprint': return { sizeBytes: 3, modifiedAt: null };
        case 'apply_track_analysis_results': return null;
        default: return null;
      }
    });
    workerConstructor.mockImplementation(() => ({
      start: vi.fn().mockResolvedValue(undefined),
      terminate: vi.fn(),
    }));
    analyzeMock.mockRejectedValue(
      new Error('abort(undefined). Build with -s ASSERTIONS=1 for more info.'),
    );
  });

  it('stops the batch after the first fatal runtime instead of duplicating it per song', async () => {
    const events: Array<{ status: string; stage: string }> = [];

    await expect(runLibraryAnalysis({
      runId: 'fatal-test',
      candidates: [1, 2, 3].map((index) => ({
        path: `/music/song-${index}.mp3`,
        name: `song-${index}.mp3`,
        sizeBytes: 3,
      })),
      resumeIncomplete: false,
      onEvent: (event) => events.push({ status: event.status, stage: event.stage }),
    })).rejects.toMatchObject({
      name: 'AnalysisWorkerFatalRuntimeError',
      stage: 'decoding',
    });

    expect(analyzeMock).toHaveBeenCalledTimes(1);
    expect(events.filter((event) => event.status === 'partial' || event.status === 'error'))
      .toHaveLength(1);
  });
});
