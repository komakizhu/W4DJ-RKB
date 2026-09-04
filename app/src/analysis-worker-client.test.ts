import { describe, expect, it, vi } from 'vitest';
import {
  AnalysisWorkerCancelledError,
  AnalysisWorkerClient,
  AnalysisWorkerFatalRuntimeError,
  AnalysisWorkerTimeoutError,
  type AnalysisWorkerLike,
} from './analysis-worker-client';
import {
  ANALYSIS_PROGRESS_STALL_TIMEOUT_MS,
  analysisTimeoutMs,
} from './analysis-timeout';
import {
  deserializeEssentiaModels,
  serializeDecodedAudio,
  serializeEssentiaModels,
  type AnalysisWorkerResponse,
} from './analysis-worker-protocol';
import { analysisErrorMessage, isFatalAnalysisRuntimeMessage } from './analysis-runtime';

class FakeWorker implements AnalysisWorkerLike {
  readonly messages: unknown[] = [];
  readonly terminated = vi.fn();
  private readonly listeners = new Map<'message' | 'error', Set<EventListener>>([
    ['message', new Set()],
    ['error', new Set()],
  ]);

  postMessage(message: unknown): void {
    this.messages.push(message);
    const request = message as { type: string; jobId: string; requestId?: string };
    if (request.type === 'start') {
      queueMicrotask(() => this.emit({ type: 'ready', jobId: request.jobId }));
    }
  }

  addEventListener(type: 'message' | 'error', listener: EventListener): void {
    this.listeners.get(type)?.add(listener);
  }

  removeEventListener(type: 'message' | 'error', listener: EventListener): void {
    this.listeners.get(type)?.delete(listener);
  }

  terminate(): void {
    this.terminated();
  }

  emit(response: AnalysisWorkerResponse): void {
    const event = new MessageEvent('message', { data: response });
    this.listeners.get('message')?.forEach((listener) => listener(event));
  }

  emitError(message: string): void {
    const event = new ErrorEvent('error', { message });
    this.listeners.get('error')?.forEach((listener) => listener(event));
  }
}

const model = {
  id: 'musicnn_embedding',
  modelJson: JSON.stringify({ modelTopology: {}, weightsManifest: [] }),
  weightData: [1, 2, 3],
  classes: [],
  kind: 'embedding' as const,
  version: 'test',
};

const audio = {
  sampleRate: 44100,
  duration: 0.01,
  channels: [new Float32Array([0, 0.1, 0])],
  musicnnSignal: null,
};

describe('AnalysisWorkerClient', () => {
  it('recognizes WebAssembly RuntimeError messages without flagging ordinary errors', () => {
    const runtimeError = new WebAssembly.RuntimeError('unreachable');
    expect(isFatalAnalysisRuntimeMessage(analysisErrorMessage(runtimeError))).toBe(true);
    expect(isFatalAnalysisRuntimeMessage('模型输入尺寸不匹配')).toBe(false);
  });

  it('preserves the worker stage and surfaces WASM aborts as fatal runtime errors', async () => {
    const worker = new FakeWorker();
    const client = new AnalysisWorkerClient(() => worker);
    await client.start('job-fatal', []);
    const pending = client.analyze({
      jobId: 'job-fatal',
      path: '/music/long-song.mp3',
      neteaseFilenameFormat: 'title_artist',
      audio,
    });
    const request = worker.messages.at(-1) as { requestId: string };
    worker.emit({
      type: 'error',
      jobId: 'job-fatal',
      requestId: request.requestId,
      stage: 'analyzingBasic',
      message: 'abort(undefined). Build with -s ASSERTIONS=1 for more info.',
    } as AnalysisWorkerResponse);

    await expect(pending).rejects.toMatchObject({
      name: 'AnalysisWorkerFatalRuntimeError',
      stage: 'analyzingBasic',
    } satisfies Partial<AnalysisWorkerFatalRuntimeError>);
    expect(worker.terminated).toHaveBeenCalledTimes(1);
  });

  it('uses the duration-based total timeout policy without a long-track cap', () => {
    expect(analysisTimeoutMs(Number.NaN)).toBe(300_000);
    expect(analysisTimeoutMs(30)).toBe(300_000);
    expect(analysisTimeoutMs(200)).toBe(660_000);
    expect(analysisTimeoutMs(1000)).toBe(3_060_000);
    expect(analysisTimeoutMs(1108.866)).toBe(3_386_598);
    expect(ANALYSIS_PROGRESS_STALL_TIMEOUT_MS).toBe(300_000);
  });

  it('transfers contiguous PCM backing stores without cloning them', () => {
    const channel = new Float32Array([0.1, 0.2]);
    const signal = new Float32Array([0.3, 0.4]);
    const serialized = serializeDecodedAudio({
      sampleRate: 44_100,
      duration: 1,
      channels: [channel],
      musicnnSignal: signal,
    });

    expect(serialized.payload.channels[0]).toBe(channel.buffer);
    expect(serialized.payload.musicnnSignal).toBe(signal.buffer);
    expect(serialized.transfer).toEqual([channel.buffer, signal.buffer]);
  });

  it('does not list one shared long-track buffer twice in the transfer list', () => {
    const signal = new Float32Array([0.1, 0.2]);
    const serialized = serializeDecodedAudio({
      sampleRate: 16_000,
      duration: 1,
      channels: [signal],
      musicnnSignal: signal,
      basicAnalysisMode: 'chunked',
    });

    expect(serialized.payload.basicAnalysisMode).toBe('chunked');
    expect(serialized.payload.channels[0]).toBe(serialized.payload.musicnnSignal);
    expect(serialized.transfer).toEqual([signal.buffer]);
  });

  it('transfers model weights as binary and keeps them binary in the Worker', () => {
    const bytes = new Uint8Array([1, 2, 3, 255]);
    const serialized = serializeEssentiaModels([{ ...model, weightData: bytes }]);
    expect(serialized.payload[0].weightData).not.toBe(bytes.buffer);
    expect(serialized.transfer).toEqual([serialized.payload[0].weightData]);
    expect(Array.from(bytes)).toEqual([1, 2, 3, 255]);
    const restored = deserializeEssentiaModels(serialized.payload);
    expect(restored[0].weightData).toBeInstanceOf(Uint8Array);
    expect(Array.from(restored[0].weightData)).toEqual([1, 2, 3, 255]);
  });

  it('keeps shared model bytes reusable for consecutive Workers', () => {
    const bytes = new Uint8Array([7, 8, 9]);
    const first = serializeEssentiaModels([{ ...model, weightData: bytes }]);
    const second = serializeEssentiaModels([{ ...model, weightData: bytes }]);

    expect(Array.from(bytes)).toEqual([7, 8, 9]);
    expect(first.payload[0].weightData).not.toBe(second.payload[0].weightData);
    expect(Array.from(new Uint8Array(first.payload[0].weightData))).toEqual([7, 8, 9]);
    expect(Array.from(new Uint8Array(second.payload[0].weightData))).toEqual([7, 8, 9]);
  });

  it('starts once, transfers a track, and routes progress/result by request id', async () => {
    const worker = new FakeWorker();
    const client = new AnalysisWorkerClient(() => worker);
    await client.start('job-1', [model]);

    const progress = vi.fn();
    const result = {
      path: '/music/Song.mp3',
      title: 'Song',
      artist: '',
      album: '',
      durationSeconds: 1,
      bpm: null,
      key: null,
      scale: null,
      keyStrength: null,
      integratedLoudnessLufs: null,
      loudnessRangeLu: null,
      energy: null,
      danceability: null,
      beatPositions: [],
      analyzedAt: 'now',
      analyzer: 'Essentia.js',
      analysisVersion: '0.2.0',
      highLevel: { status: 'model_missing' as const },
    };
    const pending = client.analyze({
      jobId: 'job-1',
      path: result.path,
      neteaseFilenameFormat: 'title_artist',
      audio,
      onProgress: progress,
    });
    const request = worker.messages.at(-1) as { requestId: string; jobId: string };
    worker.emit({
      type: 'progress',
      jobId: 'job-1',
      requestId: request.requestId,
      progress: { stage: 'analyzingBasic', message: 'basic' },
    });
    worker.emit({ type: 'result', jobId: 'job-1', requestId: request.requestId, analysis: result });

    await expect(pending).resolves.toEqual(result);
    expect(progress).toHaveBeenCalledWith({ stage: 'analyzingBasic', message: 'basic' });
    expect((worker.messages[0] as { models: Array<{ weightData: ArrayBuffer }> }).models[0].weightData)
      .toBeInstanceOf(ArrayBuffer);
  });

  it('terminates when model startup does not become ready within two minutes', async () => {
    vi.useFakeTimers();
    try {
      const worker = new FakeWorker();
      vi.spyOn(worker, 'postMessage').mockImplementation(() => undefined);
      const client = new AnalysisWorkerClient(() => worker);
      const pending = client.start('job-stuck-start', []);
      const rejection = expect(pending).rejects.toMatchObject({
        name: 'AnalysisWorkerTimeoutError',
        stage: 'loadingModels',
      } satisfies Partial<AnalysisWorkerTimeoutError>);
      await vi.advanceTimersByTimeAsync(120_001);
      await rejection;
      expect(worker.terminated).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('ignores stale job messages after a new job starts', async () => {
    const first = new FakeWorker();
    const second = new FakeWorker();
    const workers = [first, second];
    const client = new AnalysisWorkerClient(() => workers.shift() as FakeWorker);
    await client.start('job-1', []);
    client.terminate();
    await client.start('job-2', []);

    const progress = vi.fn();
    const pending = client.analyze({
      jobId: 'job-2',
      path: '/music/Song.mp3',
      neteaseFilenameFormat: 'title_artist',
      audio,
      onProgress: progress,
    });
    const request = second.messages.at(-1) as { requestId: string };
    second.emit({
      type: 'progress',
      jobId: 'job-1',
      requestId: 'job-1-1',
      progress: { stage: 'analyzingBasic', message: 'stale' },
    });
    expect(progress).not.toHaveBeenCalled();
    client.terminate();
    await expect(pending).rejects.toBeInstanceOf(AnalysisWorkerCancelledError);
  });

  it('terminates immediately and rejects in-flight analysis', async () => {
    const worker = new FakeWorker();
    const client = new AnalysisWorkerClient(() => worker);
    await client.start('job-1', []);
    const pending = client.analyze({
      jobId: 'job-1',
      path: '/music/Song.mp3',
      neteaseFilenameFormat: 'title_artist',
      audio,
    });
    client.terminate();
    await expect(pending).rejects.toBeInstanceOf(AnalysisWorkerCancelledError);
    expect(worker.terminated).toHaveBeenCalledTimes(1);
  });

  it('terminates the worker and rejects when a song exceeds its timeout', async () => {
    vi.useFakeTimers();
    try {
      const worker = new FakeWorker();
      const client = new AnalysisWorkerClient(() => worker);
      await client.start('job-timeout', []);
      const pending = client.analyze({
        jobId: 'job-timeout',
        path: '/music/Long Song.mp3',
        neteaseFilenameFormat: 'title_artist',
        audio,
        timeoutMs: 10,
      });
      const rejection = expect(pending).rejects.toMatchObject({
        name: 'AnalysisWorkerTimeoutError',
        path: '/music/Long Song.mp3',
        stage: 'preparing',
      } satisfies Partial<AnalysisWorkerTimeoutError>);

      await vi.advanceTimersByTimeAsync(11);
      await rejection;
      expect(worker.terminated).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('resets the progress watchdog without extending the total timeout', async () => {
    vi.useFakeTimers();
    try {
      const worker = new FakeWorker();
      const client = new AnalysisWorkerClient(() => worker);
      await client.start('job-progress', []);
      const pending = client.analyze({
        jobId: 'job-progress',
        path: '/music/Progress Song.mp3',
        neteaseFilenameFormat: 'title_artist',
        audio,
        timeoutMs: ANALYSIS_PROGRESS_STALL_TIMEOUT_MS * 3,
      });
      const request = worker.messages.at(-1) as { requestId: string };

      await vi.advanceTimersByTimeAsync(ANALYSIS_PROGRESS_STALL_TIMEOUT_MS - 1);
      expect(worker.terminated).not.toHaveBeenCalled();
      worker.emit({
        type: 'progress',
        jobId: 'job-progress',
        requestId: request.requestId,
        progress: { stage: 'analyzingHighLevel', message: 'still working' },
      });
      await vi.advanceTimersByTimeAsync(ANALYSIS_PROGRESS_STALL_TIMEOUT_MS - 1);
      expect(worker.terminated).not.toHaveBeenCalled();
      worker.emit({
        type: 'progress',
        jobId: 'job-progress',
        requestId: request.requestId,
        progress: { stage: 'analyzingHighLevel', message: 'still working' },
      });

      worker.emit({
        type: 'result',
        jobId: 'job-progress',
        requestId: request.requestId,
        analysis: {} as never,
      });
      await expect(pending).resolves.toEqual({});
      expect(worker.terminated).not.toHaveBeenCalled();
      expect(vi.getTimerCount()).toBe(0);
      client.terminate();
    } finally {
      vi.useRealTimers();
    }
  });

  it('times out after the progress watchdog stops receiving progress', async () => {
    vi.useFakeTimers();
    try {
      const worker = new FakeWorker();
      const client = new AnalysisWorkerClient(() => worker);
      await client.start('job-stalled', []);
      const pending = client.analyze({
        jobId: 'job-stalled',
        path: '/music/Stalled Song.mp3',
        neteaseFilenameFormat: 'title_artist',
        audio,
        timeoutMs: ANALYSIS_PROGRESS_STALL_TIMEOUT_MS * 2,
      });
      const request = worker.messages.at(-1) as { requestId: string };
      worker.emit({
        type: 'progress',
        jobId: 'job-stalled',
        requestId: request.requestId,
        progress: { stage: 'runningMusiCnn', message: 'extracting' },
      });

      await vi.advanceTimersByTimeAsync(ANALYSIS_PROGRESS_STALL_TIMEOUT_MS - 1);
      expect(worker.terminated).not.toHaveBeenCalled();
      const rejection = expect(pending).rejects.toMatchObject({
        name: 'AnalysisWorkerTimeoutError',
        path: '/music/Stalled Song.mp3',
        stage: 'runningMusiCnn',
      } satisfies Partial<AnalysisWorkerTimeoutError>);
      await vi.advanceTimersByTimeAsync(1);
      await rejection;
      expect(worker.terminated).toHaveBeenCalledTimes(1);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('clears both analysis timers when postMessage fails synchronously', async () => {
    vi.useFakeTimers();
    try {
      const worker = new FakeWorker();
      const originalPostMessage = worker.postMessage.bind(worker);
      vi.spyOn(worker, 'postMessage').mockImplementation((message) => {
        if ((message as { type?: string }).type === 'analyze') {
          throw new Error('post failed');
        }
        originalPostMessage(message);
      });
      const client = new AnalysisWorkerClient(() => worker);
      await client.start('job-post-failure', []);
      const pending = client.analyze({
        jobId: 'job-post-failure',
        path: '/music/Post Failure Song.mp3',
        neteaseFilenameFormat: 'title_artist',
        audio,
        timeoutMs: ANALYSIS_PROGRESS_STALL_TIMEOUT_MS * 2,
      });

      await expect(pending).rejects.toMatchObject({
        name: 'AnalysisWorkerClientError',
        stage: 'preparing',
      });
      expect(vi.getTimerCount()).toBe(0);
      client.terminate();
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps the total timeout active while progress continues', async () => {
    vi.useFakeTimers();
    try {
      const worker = new FakeWorker();
      const client = new AnalysisWorkerClient(() => worker);
      await client.start('job-deadline', []);
      const pending = client.analyze({
        jobId: 'job-deadline',
        path: '/music/Deadline Song.mp3',
        neteaseFilenameFormat: 'title_artist',
        audio,
        timeoutMs: 100,
      });
      const request = worker.messages.at(-1) as { requestId: string };
      for (const elapsed of [25, 50, 75]) {
        await vi.advanceTimersByTimeAsync(25);
        worker.emit({
          type: 'progress',
          jobId: 'job-deadline',
          requestId: request.requestId,
          progress: { stage: 'analyzingBasic', message: `progress at ${elapsed}ms` },
        });
      }
      const rejection = expect(pending).rejects.toMatchObject({
        name: 'AnalysisWorkerTimeoutError',
        path: '/music/Deadline Song.mp3',
      } satisfies Partial<AnalysisWorkerTimeoutError>);
      await vi.advanceTimersByTimeAsync(25);
      await rejection;
      expect(worker.terminated).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not let stale or unknown progress reset the active watchdog', async () => {
    vi.useFakeTimers();
    try {
      const worker = new FakeWorker();
      const client = new AnalysisWorkerClient(() => worker);
      await client.start('job-stale-progress', []);
      const progress = vi.fn();
      const pending = client.analyze({
        jobId: 'job-stale-progress',
        path: '/music/Stale Progress Song.mp3',
        neteaseFilenameFormat: 'title_artist',
        audio,
        onProgress: progress,
        timeoutMs: ANALYSIS_PROGRESS_STALL_TIMEOUT_MS * 2,
      });
      const request = worker.messages.at(-1) as { requestId: string };
      await vi.advanceTimersByTimeAsync(ANALYSIS_PROGRESS_STALL_TIMEOUT_MS - 1);
      worker.emit({
        type: 'progress',
        jobId: 'other-job',
        requestId: request.requestId,
        progress: { stage: 'analyzingBasic', message: 'stale job' },
      });
      worker.emit({
        type: 'progress',
        jobId: 'job-stale-progress',
        requestId: 'unknown-request',
        progress: { stage: 'analyzingBasic', message: 'unknown request' },
      });
      const rejection = expect(pending).rejects.toBeInstanceOf(AnalysisWorkerTimeoutError);
      await vi.advanceTimersByTimeAsync(1);
      await rejection;
      expect(progress).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
