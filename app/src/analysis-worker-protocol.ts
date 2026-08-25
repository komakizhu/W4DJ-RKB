import type {
  AnalysisFingerprint,
  DecodedAudioData,
  EssentiaModelFile,
  HighLevelAnalysis,
  NeteaseFilenameFormat,
  TrackAnalysis,
  TrackMetadata,
} from './analysis';
import { modelWeightDataBuffer } from './analysis';

/**
 * The worker receives transferable buffers rather than AudioBuffer instances.
 * AudioBuffer and OfflineAudioContext are deliberately kept on the UI thread;
 * all synchronous Essentia and TensorFlow work happens in the worker.
 */
export type SerializedDecodedAudio = {
  sampleRate: number;
  duration: number;
  channels: ArrayBuffer[];
  musicnnSignal: ArrayBuffer | null;
};

export type SerializedEssentiaModel = Omit<EssentiaModelFile, 'weightData'> & {
  weightData: ArrayBuffer;
};

export type AnalysisWorkerStartRequest = {
  type: 'start';
  jobId: string;
  models: SerializedEssentiaModel[];
};

export type AnalysisWorkerTrackRequest = {
  type: 'analyze';
  jobId: string;
  requestId: string;
  path: string;
  metadata?: TrackMetadata;
  fingerprint?: AnalysisFingerprint;
  neteaseFilenameFormat: NeteaseFilenameFormat;
  highLevel?: HighLevelAnalysis;
  audio: SerializedDecodedAudio;
};

export type AnalysisWorkerRequest = AnalysisWorkerStartRequest | AnalysisWorkerTrackRequest;

export type AnalysisWorkerProgress = {
  stage:
    | 'decoding'
    | 'persisting'
    | 'loadingModels'
    | 'analyzingBasic'
    | 'extractingMusiCnn'
    | 'runningMusiCnn'
    | 'extractingDiscogs'
    | 'runningDiscogsEmbedding'
    | 'runningDiscogsHeads'
    | 'runningEmotionHeads'
    | 'analyzingHighLevel'
    | 'completed'
    | string;
  message: string;
  processed?: number;
  total?: number;
  modelId?: string;
  modelFamily?: 'musicnn' | 'discogsEffnet' | 'emotion' | string;
  stageStartedAt?: string;
  elapsedMs?: number;
  backend?: string;
  patchCount?: number;
  tfMemory?: { numTensors?: number; numBytes?: number; unreliable?: boolean };
};

export type AnalysisWorkerReadyResponse = {
  type: 'ready';
  jobId: string;
};

export type AnalysisWorkerProgressResponse = {
  type: 'progress';
  jobId: string;
  requestId?: string;
  progress: AnalysisWorkerProgress;
};

export type AnalysisWorkerResultResponse = {
  type: 'result';
  jobId: string;
  requestId: string;
  analysis: TrackAnalysis;
};

export type AnalysisWorkerErrorResponse = {
  type: 'error';
  jobId: string;
  requestId?: string;
  message: string;
};

export type AnalysisWorkerResponse =
  | AnalysisWorkerReadyResponse
  | AnalysisWorkerProgressResponse
  | AnalysisWorkerResultResponse
  | AnalysisWorkerErrorResponse;

function transferableBuffer(view: Float32Array): ArrayBuffer {
  const buffer = view.buffer;
  if (buffer instanceof ArrayBuffer
    && view.byteOffset === 0
    && view.byteLength === buffer.byteLength) {
    return buffer;
  }
  return view.slice().buffer as ArrayBuffer;
}

export function serializeDecodedAudio(audio: DecodedAudioData): {
  payload: SerializedDecodedAudio;
  transfer: Transferable[];
} {
  // The decoded buffers are single-use inputs for one Worker job. Transfer
  // their backing stores directly whenever they are contiguous; only a
  // subarray or SharedArrayBuffer needs a defensive copy.
  const channels = audio.channels.map(transferableBuffer);
  const musicnnSignal = audio.musicnnSignal ? transferableBuffer(audio.musicnnSignal) : null;
  return {
    payload: {
      sampleRate: audio.sampleRate,
      duration: audio.duration,
      channels,
      musicnnSignal,
    },
    transfer: [
      ...channels,
      ...(musicnnSignal ? [musicnnSignal] : []),
    ],
  };
}

export function deserializeDecodedAudio(audio: SerializedDecodedAudio): DecodedAudioData {
  return {
    sampleRate: audio.sampleRate,
    duration: audio.duration,
    channels: audio.channels.map((channel) => new Float32Array(channel)),
    musicnnSignal: audio.musicnnSignal ? new Float32Array(audio.musicnnSignal) : null,
  };
}

export function serializeEssentiaModels(models: EssentiaModelFile[]): {
  payload: SerializedEssentiaModel[];
  transfer: Transferable[];
} {
  // Model files are reused across the per-song Worker lifecycle.  Do not
  // transfer the caller's backing store directly: postMessage detaches every
  // transferred ArrayBuffer, which would leave the shared model list unusable
  // for the next song and can crash WebKit while starting its Worker.  Audio
  // PCM remains zero-copy; model weights are small enough to copy per Worker.
  const payload = models.map((model) => {
    const source = modelWeightDataBuffer(model.weightData);
    const copy = new Uint8Array(source).slice().buffer;
    return {
      ...model,
      weightData: copy,
    };
  });
  return {
    payload,
    transfer: payload.map((model) => model.weightData),
  };
}

export function deserializeEssentiaModels(models: SerializedEssentiaModel[]): EssentiaModelFile[] {
  return models.map((model) => ({
    ...model,
    // Keep the transferred bytes binary inside the Worker.  Converting a
    // multi-megabyte model to a JS number[] here can block the Worker before
    // it emits `ready`, making every enhanced analysis hit the startup
    // timeout even though the model itself is valid.
    weightData: new Uint8Array(model.weightData),
  }));
}
