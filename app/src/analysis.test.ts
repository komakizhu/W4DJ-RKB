import { describe, expect, it } from 'vitest';
import {
  TRACK_ANALYSIS_VERSION,
  assessTrackAnalysisCompleteness,
  batchMusiCnnMelBuffer,
  batchMusiCnnMelRows,
  musicCnnInferenceBatches,
  MUSICCNN_CPU_INFERENCE_BATCH_SIZE,
  MUSICCNN_INFERENCE_BATCH_SIZE,
  padMusiCnnInferenceBatch,
  computeDiscogsEffnetMelBatches,
  streamDiscogsEffnetMelBatches,
  computeMusiCnnMelRows,
  analyzeChunkedBasicAudio,
  configureTensorflowBackend,
  deriveBroadGenreFromMsdTags,
  filterHighLevelLabels,
  filenameIdentity,
  resolveTrackMetadata,
  selectDropBeatWindow,
  shouldUseCpuTensorflowBackend,
  normalizeEssentiaModel,
  executeEssentiaModel,
  planAnalysisAudio,
  type HighLevelAnalysis,
} from './analysis';

describe('Essentia model IPC payloads', () => {
  it('puts a 1108-second stereo track on the bounded full-track analysis path', () => {
    expect(planAnalysisAudio({
      durationSeconds: 1108.866,
      sampleRate: 44_100,
      channelCount: 2,
    })).toEqual({
      mode: 'chunked',
      sampleRate: 16_000,
      channelCount: 1,
    });
  });

  it('keeps ordinary tracks on the native-resolution path', () => {
    expect(planAnalysisAudio({
      durationSeconds: 240,
      sampleRate: 44_100,
      channelCount: 2,
    })).toEqual({
      mode: 'native',
      sampleRate: 44_100,
      channelCount: 2,
    });
  });

  it('decodes compact base64 weights without retaining the wire string', () => {
    const encoded = btoa(String.fromCharCode(0, 7, 128, 255));
    const model = normalizeEssentiaModel({
      id: 'musicnn_embedding',
      modelJson: '{}',
      weightDataBase64: encoded,
      classes: [],
      kind: 'embedding',
      outputName: 'output',
      outputUnits: 4,
      version: 'test',
    });
    expect(Array.from(model.weightData)).toEqual([0, 7, 128, 255]);
    expect('weightDataBase64' in model).toBe(false);
  });

  it('scopes graph execution so intermediate tensors are released', () => {
    const calls: string[] = [];
    const output = { dispose: () => calls.push('output-dispose') };
    const tf = {
      tidy: (run: () => unknown) => {
        calls.push('tidy-start');
        const value = run();
        calls.push('tidy-end');
        return value;
      },
      tensor: () => ({ dispose: () => calls.push('training-dispose') }),
    };
    const model = {
      executor: { inputs: [{}, {}] },
      execute: (inputs: unknown[]) => {
        expect(inputs).toHaveLength(2);
        return output;
      },
    };
    expect(executeEssentiaModel(tf, model, { id: 'features' })).toBe(output);
    expect(calls).toEqual(['tidy-start', 'training-dispose', 'tidy-end']);
  });
});

describe('MusiCNN broad genre projection', () => {
  it('releases native frame and Mel vectors while preserving row order', async () => {
    const deleted = { frames: 0, frame: 0, bands: 0 };
    const rows = [[1, 2], [3, 4], [5, 6]];
    const frames = {
      size: () => rows.length,
      get: (index: number) => ({
        index,
        delete: () => { deleted.frame += 1; },
      }),
      delete: () => { deleted.frames += 1; },
    };
    const essentia = {
      FrameGenerator: () => frames,
      TensorflowInputMusiCNN: (frame: { index: number }) => ({
        bands: {
          values: rows[frame.index],
          delete: () => { deleted.bands += 1; },
        },
      }),
      vectorToArray: (value: { values: number[] }) => Float32Array.from(value.values),
    } as never;
    const progress: Array<{ processed: number; total: number }> = [];

    await expect(computeMusiCnnMelRows(essentia, new Float32Array(2048), (value) => {
      progress.push(value);
    })).resolves.toMatchObject({
      melRows: rows,
      patchSize: 187,
      melBands: 96,
      frameCount: rows.length,
    });
    expect(deleted).toEqual({ frames: 1, frame: rows.length, bands: rows.length });
    expect(progress).toEqual([{ processed: rows.length, total: rows.length }]);
  });

  it('uses bounded extraction-local Essentia instances with bounded frame generators', async () => {
    const signal = new Float32Array((256 * 2) * 256 + 512);
    const lifecycle = { created: 0, deleted: 0, generators: 0 };
    let nextFrameIndex = 0;
    let nextOutputIndex = 0;
    const makeEssentia = () => {
      lifecycle.created += 1;
      return {
        arrayToVector: () => ({
          set: () => undefined,
          delete: () => undefined,
        }),
        FrameGenerator: (audio: Float32Array) => {
          const count = Math.max(0, Math.floor((audio.length - 512) / 256) + 1);
          const base = nextFrameIndex;
          nextFrameIndex += count;
          lifecycle.generators += 1;
          return {
            size: () => count,
            get: (index: number) => ({
              index: base + index,
              delete: () => undefined,
            }),
            delete: () => undefined,
          };
        },
        TensorflowInputMusiCNN: () => {
          const index = nextOutputIndex++;
          return {
            bands: {
              values: [index, index + 0.5],
              delete: () => undefined,
            },
          };
        },
        vectorToArray: (value: { values: number[] }) => Float32Array.from(value.values),
        delete: () => { lifecycle.deleted += 1; },
      } as never;
    };
    const essentia = makeEssentia();
    essentia.createInstance = makeEssentia;
    const result = await computeMusiCnnMelRows(essentia, signal);
    expect(result.frameCount).toBe(513);
    expect(result.melRows[0]).toEqual([0, 0.5]);
    expect(result.melRows.at(-1)).toEqual([512, 512.5]);
    expect(lifecycle.created).toBe(3);
    expect(lifecycle.deleted).toBe(2);
    expect(lifecycle.generators).toBe(2);
  });

  it('copies one native MusiCNN frame at a time from bounded containers', async () => {
    const frameCount = 514;
    const signal = new Float32Array(512 + (frameCount - 1) * 256);
    const lifecycle = { created: 0, deleted: 0, shutdowns: 0, reinstantiates: 0, generators: 0 };
    const makeEssentia = () => {
      lifecycle.created += 1;
      return {
        arrayToVector: () => ({
          set: () => undefined,
          delete: () => undefined,
        }),
        FrameGenerator: (audio: Float32Array) => {
          lifecycle.generators += 1;
          const count = Math.max(0, Math.floor((audio.length - 512) / 256) + 1);
          return {
            size: () => count,
            get: () => ({ delete: () => undefined }),
            delete: () => undefined,
          };
        },
        TensorflowInputMusiCNN: () => ({
          bands: {
            size: () => 1,
            get: () => 1,
            delete: () => undefined,
          },
        }),
        vectorToArray: () => Float32Array.of(1),
        shutdown: () => { lifecycle.shutdowns += 1; },
        reinstantiate: () => { lifecycle.reinstantiates += 1; },
        flushPendingDeletes: () => undefined,
        delete: () => { lifecycle.deleted += 1; },
      } as never;
    };
    const essentia = makeEssentia();
    essentia.createInstance = makeEssentia;

    const result = await computeMusiCnnMelRows(essentia, signal, undefined, { collectRows: false });

    expect(result.frameCount).toBe(frameCount);
    expect(lifecycle).toEqual({
      created: 3,
      deleted: 2,
      shutdowns: 0,
      reinstantiates: 0,
      generators: 2,
    });
  });

  it('finishes the tail while rebuilding only bounded native frame generators', async () => {
    const frameCount = 9960;
    const signal = new Float32Array(512 + (frameCount - 1) * 256);
    const lifecycle = {
      created: 0,
      deleted: 0,
      convertedFrames: 0,
      frameWrites: 0,
      flushed: 0,
      generatedFrames: 0,
      generators: 0,
    };
    const makeEssentia = () => {
      lifecycle.created += 1;
      return {
        arrayToVector: () => {
          lifecycle.convertedFrames += 1;
          return {
            index: 0,
            set: () => { lifecycle.frameWrites += 1; },
            delete: () => undefined,
          };
        },
        FrameGenerator: (audio: Float32Array) => {
          lifecycle.generators += 1;
          const count = Math.max(0, Math.floor((audio.length - 512) / 256) + 1);
          return {
            size: () => count,
            get: () => ({
                index: lifecycle.generatedFrames++,
                delete: () => undefined,
              }),
            delete: () => undefined,
          };
        },
        TensorflowInputMusiCNN: (frame: { index: number }) => ({
          bands: {
            values: [frame.index],
            delete: () => undefined,
          },
        }),
        vectorToArray: (value: { values: number[] }) => Float32Array.from(value.values),
        flushPendingDeletes: () => { lifecycle.flushed += 1; },
        delete: () => { lifecycle.deleted += 1; },
      } as never;
    };
    const essentia = makeEssentia();
    essentia.createInstance = makeEssentia;
    const progress: Array<{ processed: number; total: number }> = [];

    await expect(computeMusiCnnMelRows(essentia, signal, (value) => {
      progress.push(value);
    }, { collectRows: false })).resolves.toMatchObject({ frameCount });

    expect(progress.at(-1)).toEqual({ processed: frameCount, total: frameCount });
    expect(lifecycle).toEqual({
      created: 21,
      deleted: 20,
      convertedFrames: 0,
      frameWrites: 0,
      flushed: frameCount + 40,
      generatedFrames: frameCount,
      generators: 20,
    });
  });

  it('projects the strongest MSD tag into the existing broad genre labels', () => {
    const scores = Array.from({ length: 50 }, () => 0.01);
    scores[48] = 0.91; // House
    scores[0] = 0.4; // rock
    expect(deriveBroadGenreFromMsdTags(scores)).toEqual({
      label: 'dan',
      confidence: 0.91,
    });
  });

  it('returns null when the model output has no finite score', () => {
    expect(deriveBroadGenreFromMsdTags([])).toBeNull();
  });

  it('groups mel rows into explicit 3D patches and pads the final patch', () => {
    expect(batchMusiCnnMelRows([[1, 2], [3, 4], [5, 6]], 2, 2)).toEqual([
      [[1, 2], [3, 4]],
      [[5, 6], [0, 0]],
    ]);
  });

  it('normalizes short mel rows to the model band count', () => {
    expect(batchMusiCnnMelRows([[1]], 1, 3)).toEqual([[[1, 0, 0]]]);
  });

  it('builds a contiguous MusiCNN tensor buffer and zero-pads the tail', () => {
    const buffer = Float32Array.from([1, 2, 3, 4, 5, 6]);
    const batched = batchMusiCnnMelBuffer(buffer, 3, 2, 2);
    expect(batched.batchCount).toBe(2);
    expect(Array.from(batched.values)).toEqual([1, 2, 3, 4, 5, 6, 0, 0]);
  });

  it.each([
    { patches: 1, expected: [{ offset: 0, validPatches: 1 }] },
    {
      patches: 8,
      expected: [{ offset: 0, validPatches: 8 }],
    },
    {
      patches: 9,
      expected: [
        { offset: 0, validPatches: 8 },
        { offset: 8, validPatches: 1 },
      ],
    },
    {
      patches: 18,
      expected: [
        { offset: 0, validPatches: 8 },
        { offset: 8, validPatches: 8 },
        { offset: 16, validPatches: 2 },
      ],
    },
  ])('partitions $patches MusiCNN patches without dropping the tail', ({ patches, expected }) => {
    expect(musicCnnInferenceBatches(patches)).toEqual(expected);
  });

  it('keeps the MusiCNN tail input at the fixed eight-patch shape', () => {
    const patchStride = 2;
    const values = Float32Array.from({ length: 10 * patchStride }, (_, index) => index + 1);
    const padded = padMusiCnnInferenceBatch(values, 8, 2, patchStride, MUSICCNN_INFERENCE_BATCH_SIZE);
    expect(padded.length).toBe(MUSICCNN_INFERENCE_BATCH_SIZE * patchStride);
    expect(Array.from(padded.slice(0, 4))).toEqual([17, 18, 19, 20]);
    expect(Array.from(padded.slice(4))).toEqual([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
  });

  it('uses one-patch CPU inference groups without changing the eight-patch WebGL default', () => {
    expect(MUSICCNN_CPU_INFERENCE_BATCH_SIZE).toBe(1);
    expect(musicCnnInferenceBatches(3, MUSICCNN_CPU_INFERENCE_BATCH_SIZE)).toEqual([
      { offset: 0, validPatches: 1 },
      { offset: 1, validPatches: 1 },
      { offset: 2, validPatches: 1 },
    ]);
    expect(musicCnnInferenceBatches(10)).toHaveLength(2);
  });

  it('builds independent Discogs [N,128,96] batches and pads the tail', async () => {
    const deleted = { frames: 0, bands: 0, generator: 0 };
    const frameCount = 129;
    const frames = {
      size: () => frameCount,
      get: (index: number) => ({ index, delete: () => { deleted.frames += 1; } }),
      delete: () => { deleted.generator += 1; },
    };
    const essentia = {
      FrameGenerator: () => frames,
      TensorflowInputDiscogsEffNet: (frame: { index: number }) => ({
        bands: {
          values: Array.from({ length: 96 }, (_, band) => frame.index + band / 100),
          delete: () => { deleted.bands += 1; },
        },
      }),
      vectorToArray: (value: { values: number[] }) => Float32Array.from(value.values),
    } as never;
    const progress: Array<{ processedPatches: number; totalPatches: number }> = [];
    const batches = await computeDiscogsEffnetMelBatches(essentia, new Float32Array(512), (value) => {
      progress.push(value);
    });
    expect(batches).toHaveLength(1);
    expect(batches[0].batchSize).toBe(64);
    expect(batches[0].framesPerPatch).toBe(128);
    expect(batches[0].melBands).toBe(96);
    expect(batches[0].validPatches).toBe(2);
    expect(batches[0].values.length).toBe(64 * 128 * 96);
    expect(batches[0].values[0]).toBeCloseTo(0);
    expect(batches[0].values[128 * 96]).toBeCloseTo(128);
    // The unused tail of the final 64-patch batch is explicitly zero-filled.
    expect(batches[0].values[2 * 128 * 96]).toBe(0);
    expect(deleted).toEqual({ frames: frameCount, bands: frameCount, generator: 1 });
    expect(progress.at(-1)).toEqual({ processedPatches: 2, totalPatches: 2 });
  });

  it('keeps the production Discogs tail at the fixed bs64 tensor size', async () => {
    const frameCount = 129;
    const signal = new Float32Array(512 + (frameCount - 1) * 256);
    const batches = [] as Array<{ values: Float32Array; validPatches: number }>;
    for await (const batch of streamDiscogsEffnetMelBatches(
      {} as never,
      signal,
      undefined,
      { tensorRuntime: { signal: { stft: true } } },
    )) {
      batches.push(batch);
    }
    expect(batches).toHaveLength(1);
    expect(batches[0].validPatches).toBe(2);
    expect(batches[0].values.length).toBe(64 * 128 * 96);
    expect(batches[0].values[2 * 128 * 96]).toBe(0);
  });

  it('reuses one production Discogs frame without native frame generators', async () => {
    const frameCount = 300;
    const signal = new Float32Array(512 + (frameCount - 1) * 256);
    const lifecycle = {
      created: 0,
      deleted: 0,
      flushed: 0,
      frameVectors: 0,
      frameWrites: 0,
      generators: 0,
    };
    let nextOutputIndex = 0;
    const makeEssentia = () => {
      lifecycle.created += 1;
      return {
        arrayToVector: () => {
          lifecycle.frameVectors += 1;
          return {
            set: () => { lifecycle.frameWrites += 1; },
            delete: () => undefined,
          };
        },
        FrameGenerator: (audio: Float32Array) => {
          lifecycle.generators += 1;
          const count = Math.max(0, Math.floor((audio.length - 512) / 256) + 1);
          return {
            size: () => count,
            get: (index: number) => ({ index, delete: () => undefined }),
            delete: () => undefined,
          };
        },
        TensorflowInputDiscogsEffNet: () => {
          const index = nextOutputIndex++;
          return {
            bands: {
              values: Array.from({ length: 96 }, (_, band) => index + band / 100),
              delete: () => undefined,
            },
          };
        },
        vectorToArray: (value: { values: number[] }) => Float32Array.from(value.values),
        flushPendingDeletes: () => { lifecycle.flushed += 1; },
        delete: () => { lifecycle.deleted += 1; },
      } as never;
    };
    const essentia = makeEssentia();
    essentia.createInstance = makeEssentia;

    const batches = await computeDiscogsEffnetMelBatches(essentia, signal);

    expect(batches).toHaveLength(1);
    expect(batches[0].validPatches).toBe(3);
    expect(lifecycle).toEqual({
      created: 2,
      deleted: 1,
      flushed: frameCount + 2,
      frameVectors: 0,
      frameWrites: 0,
      generators: 1,
    });
  });

  it('processes every basic-analysis chunk and translates beats to track time', async () => {
    const signal = Float32Array.from({ length: 65 * 10 }, (_, index) => index % 10 / 10);
    const essentia = {
      arrayToVector: (input: Float32Array) => ({ input, delete: () => undefined }),
      vectorToArray: (value: number[] | Float32Array) => Float32Array.from(value),
      RhythmExtractor2013: () => ({ bpm: 120, ticks: [1, 2] }),
      KeyExtractor: () => ({ key: 'C', scale: 'major', strength: 0.8 }),
      LoudnessEBUR128: () => ({ integratedLoudness: -12, loudnessRange: 4 }),
      Danceability: () => ({ danceability: 0.5 }),
      BeatsLoudness: () => ({ loudness: [-3, -4] }),
    } as never;
    const progress: Array<{ processed?: number; total?: number }> = [];

    const result = await analyzeChunkedBasicAudio(essentia, {
      sampleRate: 10,
      duration: 65,
      channels: [signal],
      musicnnSignal: null,
      basicAnalysisMode: 'chunked',
    }, (event) => progress.push(event));

    expect(result.bpm).toBe(120);
    expect(result.key).toBe('C');
    expect(result.integratedLoudnessLufs).toBeCloseTo(-12);
    expect(result.energy).toBeGreaterThan(0);
    expect(result.beatPositions).toEqual([1, 2, 31, 32, 61, 62]);
    expect(progress.at(-1)).toMatchObject({ processed: signal.length, total: signal.length });
  });
});

describe('TensorFlow.js backend selection', () => {
  it('selects the native WebKit path but leaves Chromium WebGL selection alone', () => {
    expect(shouldUseCpuTensorflowBackend(
      'Mozilla/5.0 (Macintosh; Intel Mac OS X) AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15',
    )).toBe(true);
    expect(shouldUseCpuTensorflowBackend(
      'Mozilla/5.0 (Macintosh; Intel Mac OS X) AppleWebKit/537.36 Chrome/139.0.0.0 Safari/537.36',
    )).toBe(false);
    expect(shouldUseCpuTensorflowBackend('Mozilla/5.0 Firefox/142.0')).toBe(false);
  });

  it('waits for the selected backend before returning its name', async () => {
    const calls: string[] = [];
    let backend = 'webgl';
    const tf = {
      setBackend: async (name: string) => {
        calls.push(`set:${name}`);
        backend = name;
        return true;
      },
      ready: async () => { calls.push('ready'); },
      getBackend: () => backend,
    };
    await expect(configureTensorflowBackend(tf,
      'Mozilla/5.0 AppleWebKit/605.1.15 Version/18.0 Safari/605.1.15'))
      .resolves.toBe('wasm');
    expect(calls).toEqual(['set:wasm', 'ready']);
  });
});

describe('analysis filename identity', () => {
  it('uses the existing Artist - Song convention for ordinary files', () => {
    expect(filenameIdentity('/music/Artist - Song.mp3')).toEqual({
      title: 'Song',
      artist: 'Artist',
      album: '',
    });
  });

  it('follows the selected NetEase filename order for NCM files', () => {
    expect(filenameIdentity('/music/网易云/Song - Artist.ncm', 'title_artist')).toMatchObject({
      title: 'Song',
      artist: 'Artist',
    });
    expect(filenameIdentity('/music/网易云/Artist - Song.ncm', 'artist_title')).toMatchObject({
      title: 'Song',
      artist: 'Artist',
    });
    expect(filenameIdentity('/music/网易云/Song.ncm', 'title_only')).toMatchObject({
      title: 'Song',
      artist: '',
    });
  });

  it('repairs metadata that is the exact reverse of the filename identity', () => {
    expect(resolveTrackMetadata('/music/Artist - Song.mp3', {
      title: 'Artist',
      artist: 'Song',
      album: 'Album',
    })).toEqual({
      title: 'Song',
      artist: 'Artist',
      album: 'Album',
    });
  });

  it('keeps valid metadata while filling missing fields from the filename', () => {
    expect(resolveTrackMetadata('/music/Artist - Song.mp3', {
      title: 'Tagged title',
      artist: '',
      album: '',
    })).toEqual({
      title: 'Tagged title',
      artist: 'Artist',
      album: '',
    });
  });

  it('bumps the cache schema when the fingerprinted identity changes', () => {
    expect(TRACK_ANALYSIS_VERSION).toBe('0.2.0');
  });
});

describe('Drop loudness selection', () => {
  it('selects the loudest contiguous 32-beat window after head and tail exclusion', () => {
    const beats = Array.from({ length: 80 }, (_, index) => index);
    const loudness = beats.map((_, index) => (index >= 30 && index < 62 ? -3 : -12));
    expect(selectDropBeatWindow(beats, loudness, 80)).toEqual({
      startIndex: 30,
      endIndex: 61,
      averageLoudness: -3,
    });
  });

  it('skips when the song has fewer than 32 eligible beats', () => {
    expect(selectDropBeatWindow([0, 1, 2], [-1, -2, -3], 3)).toBeNull();
  });

  it('keeps beat positions paired with their matching loudness values', () => {
    expect(selectDropBeatWindow(
      [0, 1, 2, Number.NaN, 3],
      [-10, -10, -10, 100, -2],
      4,
      2,
    )).toEqual({
      startIndex: 2,
      endIndex: 4,
      averageLoudness: -6,
    });
  });
});

describe('Essentia high-level label filtering', () => {
  it('keeps multiple positive labels and filters negative or weak labels', () => {
    expect(filterHighLevelLabels([
      { label: 'aggressive', confidence: 0.9 },
      { label: 'happy', confidence: 0.8 },
      { label: 'non_sad', confidence: 0.99 },
      { label: 'relaxed', confidence: 0.74 },
    ])).toEqual({
      accepted: [
        { label: 'aggressive', confidence: 0.9 },
        { label: 'happy', confidence: 0.8 },
      ],
      filtered: [
        { label: 'non_sad', confidence: 0.99, reason: 'negative_label' },
        { label: 'relaxed', confidence: 0.74, reason: 'below_threshold' },
      ],
    });
  });

  it('serializes unavailable diagnostic confidence as null instead of NaN', () => {
    const result = filterHighLevelLabels([{ label: 'missing-model-output', confidence: Number.NaN }]);
    expect(result.filtered).toEqual([{
      label: 'missing-model-output',
      confidence: null,
      reason: 'below_threshold',
    }]);
    expect(JSON.parse(JSON.stringify(result.filtered))).toEqual(result.filtered);
  });
});

describe('high-level emotion JSON compatibility', () => {
  it('keeps old five-Mood JSON readable while accepting new camelCase fields', () => {
    const oldJson = JSON.stringify({
      status: 'completed',
      modelVersion: 'legacy',
      genre: [],
      mood: [{ label: 'happy', confidence: 0.9 }],
      instrument: [],
      filtered: [],
    });
    const old = JSON.parse(oldJson) as HighLevelAnalysis;
    expect(old.mood).toEqual([{ label: 'happy', confidence: 0.9 }]);
    expect(old.style).toBeUndefined();
    expect(old.emotionCandidates).toBeUndefined();
    expect(old.moodCluster).toBeUndefined();

    const current: HighLevelAnalysis = {
      status: 'completed',
      modelVersion: 'emotion-v1',
      genre: [],
      style: [{ label: '80s', confidence: 0.8 }],
      mood: [{ label: 'happy', confidence: 0.9 }],
      instrument: [],
      emotionCandidates: {
        emomusic: { model: 'emomusic', status: 'completed', valence: 7, arousal: 6 },
        muse: { model: 'muse', status: 'model_missing', valence: null, arousal: null, reason: 'missing' },
      },
      moodCluster: [{ label: 'passionate', confidence: 0.7 }],
      moodClusterStatus: 'completed',
    };
    const roundTrip = JSON.parse(JSON.stringify(current)) as HighLevelAnalysis;
    expect(roundTrip.emotionCandidates?.emomusic?.valence).toBe(7);
    expect(roundTrip.emotionCandidates?.muse?.status).toBe('model_missing');
    expect(roundTrip.moodCluster?.[0].label).toBe('passionate');
  });
});

describe('analysis completion contract', () => {
  it('requires the core metadata values before counting a track as basic complete', () => {
    const base = {
      path: '/music/song.mp3',
      title: 'Song',
      artist: 'Artist',
      album: '',
      durationSeconds: 180,
      bpm: 120,
      key: 'C',
      scale: 'major',
      keyStrength: 0.8,
      integratedLoudnessLufs: -12,
      loudnessRangeLu: 4,
      energy: 0.4,
      danceability: 0.7,
      beatPositions: [],
      analyzedAt: '2026-08-23T00:00:00Z',
      analyzer: 'Essentia.js',
      analysisVersion: TRACK_ANALYSIS_VERSION,
    };
    expect(assessTrackAnalysisCompleteness(base).basicComplete).toBe(true);
    expect(assessTrackAnalysisCompleteness({ ...base, key: null }).basicComplete).toBe(false);
    expect(assessTrackAnalysisCompleteness({ ...base, integratedLoudnessLufs: null }).basicComplete).toBe(false);
  });
});
