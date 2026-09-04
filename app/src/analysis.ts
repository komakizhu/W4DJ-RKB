import type { AnalysisWorkerProgress } from './analysis-worker-protocol';
import { analysisTimeoutMs } from './analysis-timeout';
import { runEmotionHeads } from './emotion-models';
import { runDiscogsEffnetHeads } from './discogs-effnet';
import { analysisErrorMessage, isFatalAnalysisRuntimeMessage } from './analysis-runtime';
import wasmBinaryUrl from '@tensorflow/tfjs-backend-wasm/wasm-out/tfjs-backend-wasm.wasm?url';
import wasmSimdBinaryUrl from '@tensorflow/tfjs-backend-wasm/wasm-out/tfjs-backend-wasm-simd.wasm?url';
import wasmThreadedSimdBinaryUrl from '@tensorflow/tfjs-backend-wasm/wasm-out/tfjs-backend-wasm-threaded-simd.wasm?url';

export type TrackAnalysis = {
  path: string;
  title: string;
  artist: string;
  album: string;
  genre?: string | null;
  durationSeconds: number | null;
  bpm: number | null;
  key: string | null;
  scale: string | null;
  keyStrength: number | null;
  integratedLoudnessLufs: number | null;
  loudnessRangeLu: number | null;
  energy: number | null;
  danceability: number | null;
  beatPositions: number[];
  analyzedAt: string;
  analyzer: string;
  analysisVersion: string;
  sourceSizeBytes?: number | null;
  sourceModifiedAt?: number | null;
  sourceFilenameFormat?: NeteaseFilenameFormat | null;
  dropLoudnessLufs?: number | null;
  dropAnalysis?: DropAnalysisDetails | null;
  highLevel?: HighLevelAnalysis | null;
};

export type DropAnalysisDetails = {
  status: 'completed' | 'skipped' | 'failed';
  reason?: string | null;
  beatStartIndex?: number | null;
  beatEndIndex?: number | null;
  beatCount?: number | null;
  segmentStartSeconds?: number | null;
  segmentEndSeconds?: number | null;
  selectedAverageBeatLoudness?: number | null;
};

export type AnalysisLabel = {
  label: string;
  confidence: number;
};

export type EmotionHeadStatus = 'completed' | 'model_missing' | 'failed' | 'cancelled' | 'timeout';

export type DiscogsEffnetHeadId =
  | 'moodTheme'
  | 'approachability'
  | 'instrumentation'
  | 'timbre'
  | 'danceability';

export type DiscogsEffnetHeadStatus =
  | 'completed'
  | 'model_missing'
  | 'failed'
  | 'cancelled'
  | 'timeout';

export type DiscogsEffnetHeadResult = {
  model: DiscogsEffnetHeadId;
  status: DiscogsEffnetHeadStatus;
  version: string;
  labels: AnalysisLabel[];
  scores: Record<string, number>;
  frameCount: number;
  threshold?: number;
  selectedClass?: string;
  selectedConfidence?: number;
  reason?: string | null;
};

export type DiscogsEffnetAnalysis = {
  embeddingModel: 'discogs-effnet-bs64-1';
  embeddingDimensions: 1280;
  inputShape: [number, number, number];
  heads: Partial<Record<DiscogsEffnetHeadId, DiscogsEffnetHeadResult>>;
};

export type ContinuousEmotionResult = {
  model: 'emomusic' | 'muse';
  status: EmotionHeadStatus;
  valence: number | null;
  arousal: number | null;
  reason?: string | null;
};

export type EmotionCandidates = {
  emomusic?: ContinuousEmotionResult;
  muse?: ContinuousEmotionResult;
};

export type HighLevelAnalysis = {
  status: 'completed' | 'model_missing' | 'failed';
  modelVersion?: string | null;
  reason?: string | null;
  genre?: AnalysisLabel[];
  style?: AnalysisLabel[];
  mood?: AnalysisLabel[];
  instrument?: AnalysisLabel[];
  emotionCandidates?: EmotionCandidates;
  moodCluster?: AnalysisLabel[];
  moodClusterStatus?: EmotionHeadStatus;
  moodClusterReason?: string | null;
  filtered?: Array<{ label: string; confidence: number | null; reason: string }>;
  discogsEffnet?: DiscogsEffnetAnalysis;
};

export const REQUIRED_DISCOGS_HEAD_IDS: readonly DiscogsEffnetHeadId[] = [
  'moodTheme',
  'approachability',
  'instrumentation',
  'timbre',
  'danceability',
];

/**
 * The enhanced analysis contract is intentionally strict.  A basic
 * Essentia result can still be persisted as a partial result, but it is not
 * reusable or counted as a completed song until every configured high-level
 * stage has produced a terminal success value.
 */
export type TrackAnalysisCompleteness = {
  complete: boolean;
  basicComplete: boolean;
  highLevelComplete: boolean;
  reasons: string[];
  discogsCompletedHeads: number;
  discogsTotalHeads: number;
};

export function isBasicTrackAnalysisComplete(entry: TrackAnalysis | undefined): boolean {
  if (!entry) return false;
  const numericFields = [
    entry.durationSeconds,
    entry.bpm,
    entry.integratedLoudnessLufs,
    entry.energy,
    entry.danceability,
  ];
  return numericFields.every((value) => typeof value === 'number' && Number.isFinite(value))
    && typeof entry.key === 'string'
    && entry.key.trim().length > 0;
}

export function assessTrackAnalysisCompleteness(
  entry: TrackAnalysis | undefined,
): TrackAnalysisCompleteness {
  const reasons: string[] = [];
  const highLevelReasons: string[] = [];
  const basicComplete = isBasicTrackAnalysisComplete(entry);
  if (!basicComplete) reasons.push('基础分析未完成');
  const highLevel = entry?.highLevel;
  let discogsCompletedHeads = 0;
  const discogsTotalHeads = REQUIRED_DISCOGS_HEAD_IDS.length;
  if (!highLevel) {
    highLevelReasons.push('未生成高级分析');
  } else {
    if (highLevel.status !== 'completed') {
      highLevelReasons.push(`高级分析状态为 ${highLevel.status}`);
    }
    const dropStatus = entry?.dropAnalysis?.status;
    if (dropStatus === 'failed' || !dropStatus) {
      highLevelReasons.push('Drop 分析未完成');
    }
    const discogs = highLevel.discogsEffnet;
    if (!discogs) {
      highLevelReasons.push('Discogs-EffNet embedding 未完成');
    } else {
      for (const id of REQUIRED_DISCOGS_HEAD_IDS) {
        if (discogs.heads[id]?.status === 'completed') {
          discogsCompletedHeads += 1;
        } else {
          highLevelReasons.push(`Discogs head ${id} 未完成`);
        }
      }
    }
    const emotionCandidates = highLevel.emotionCandidates;
    for (const id of ['emomusic', 'muse'] as const) {
      if (emotionCandidates?.[id]?.status !== 'completed') {
        highLevelReasons.push(`情绪模型 ${id} 未完成`);
      }
    }
    if (highLevel.moodClusterStatus !== 'completed') {
      highLevelReasons.push('MIREX 情绪簇未完成');
    }
  }
  reasons.push(...highLevelReasons);
  const highLevelComplete = highLevelReasons.length === 0;
  return {
    complete: basicComplete && highLevelComplete,
    basicComplete,
    highLevelComplete,
    reasons,
    discogsCompletedHeads,
    discogsTotalHeads,
  };
}

export function isCompleteTrackAnalysis(entry: TrackAnalysis | undefined): boolean {
  return assessTrackAnalysisCompleteness(entry).complete;
}

function highLevelAnalysisHasRequiredOutputs(value: HighLevelAnalysis): boolean {
  if (!value.discogsEffnet) return false;
  if (REQUIRED_DISCOGS_HEAD_IDS.some((id) => value.discogsEffnet?.heads[id]?.status !== 'completed')) {
    return false;
  }
  return value.emotionCandidates?.emomusic?.status === 'completed'
    && value.emotionCandidates?.muse?.status === 'completed'
    && value.moodClusterStatus === 'completed';
}

export type DiscogsEffnetMelProgress = {
  processedPatches: number;
  totalPatches: number;
};

export type DiscogsEffnetMelBatch = {
  values: Float32Array;
  batchSize: number;
  framesPerPatch: 128;
  melBands: 96;
  validPatches: number;
};

export type EssentiaModelSpec = {
  id: string;
  kind: 'embedding' | 'genreEmbedding' | 'genre' | 'mood' | 'instrument' | 'emotionContinuous' | 'emotionCluster' | 'discogsEffnetEmbedding' | 'discogsEffnetHead';
  inputWidth: 200 | 1280 | null;
  inputShape?: readonly number[] | null;
  outputUnits: number;
  outputName: string;
  classes: readonly string[];
  version: string;
};

export type EssentiaModelFile = {
  id: string;
  modelJson: string;
  /**
   * Tauri returns JSON byte arrays, while the Worker keeps transferred model
   * weights as a Uint8Array.  Accept both without forcing a large model to
   * make an additional number[] copy at every boundary.
   */
  weightData: number[] | Uint8Array;
  classes: string[];
  kind: EssentiaModelSpec['kind'];
  outputName?: string;
  outputUnits?: number | null;
  inputShape?: number[] | null;
  embeddingFamily?: string | null;
  inputWidth?: number | null;
  version: string;
};

/** Wire shape returned by Tauri.  Model weights use base64 to avoid a huge
 * JSON number array blocking WebKit while the IPC response is materialized. */
export type EssentiaModelWire = Omit<EssentiaModelFile, 'weightData'> & {
  weightData?: number[] | Uint8Array;
  weightDataBase64?: string;
};

export function normalizeEssentiaModel(model: EssentiaModelWire): EssentiaModelFile {
  if (model.weightData !== undefined) {
    return model as EssentiaModelFile;
  }
  if (!model.weightDataBase64) {
    throw new Error(`Essentia 模型 ${model.id} 缺少权重数据`);
  }
  const binary = globalThis.atob(model.weightDataBase64);
  const weightData = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    weightData[index] = binary.charCodeAt(index);
  }
  const { weightDataBase64: _weightDataBase64, ...metadata } = model;
  return { ...metadata, weightData } as EssentiaModelFile;
}

export function modelWeightDataBuffer(weightData: number[] | Uint8Array): ArrayBuffer {
  if (weightData instanceof Uint8Array) {
    if (weightData.byteOffset === 0 && weightData.byteLength === weightData.buffer.byteLength) {
      return weightData.buffer as ArrayBuffer;
    }
    return weightData.slice().buffer;
  }
  return Uint8Array.from(weightData).buffer;
}

/**
 * Audio data that can cross the analysis Worker boundary. The browser-only
 * decode/resample steps stay on the UI thread; the Worker receives detached
 * PCM buffers and performs all synchronous Essentia/TensorFlow work.
 */
export type DecodedAudioData = {
  sampleRate: number;
  duration: number;
  channels: Float32Array[];
  musicnnSignal: Float32Array | null;
  /** Oversized sources are represented as a bounded mono signal and analyzed
   * in complete time chunks inside the Worker. */
  basicAnalysisMode?: 'chunked';
};

export const BOUNDED_ANALYSIS_SAMPLE_RATE = 16_000;
export const LONG_TRACK_DURATION_THRESHOLD_SECONDS = 600;
export const MAX_DECODED_PCM_BYTES = 128 * 1024 * 1024;

export type AnalysisAudioPlanInput = {
  durationSeconds: number;
  sampleRate: number;
  channelCount: number;
};

export type AnalysisAudioPlan = {
  mode: 'native' | 'chunked';
  sampleRate: number;
  channelCount: number;
};

/**
 * Keep the browser/Worker boundary below a predictable PCM budget. A long
 * source is not truncated: it is downmixed to the model rate and the Worker
 * runs the basic algorithms over every chunk of that signal.
 */
export function planAnalysisAudio(input: AnalysisAudioPlanInput): AnalysisAudioPlan {
  const durationSeconds = Number(input.durationSeconds);
  const sampleRate = Math.max(1, Math.trunc(Number(input.sampleRate)) || 44_100);
  const channelCount = Math.min(Math.max(1, Math.trunc(Number(input.channelCount)) || 1), 2);
  const decodedPcmBytes = Number.isFinite(durationSeconds) && durationSeconds > 0
    ? durationSeconds * sampleRate * channelCount * Float32Array.BYTES_PER_ELEMENT
    : 0;
  if (durationSeconds >= LONG_TRACK_DURATION_THRESHOLD_SECONDS
    || decodedPcmBytes > MAX_DECODED_PCM_BYTES) {
    return {
      mode: 'chunked',
      sampleRate: BOUNDED_ANALYSIS_SAMPLE_RATE,
      channelCount: 1,
    };
  }
  return { mode: 'native', sampleRate, channelCount };
}

export type AnalysisWorkerClientLike = {
  analyze: (request: {
    jobId: string;
    path: string;
    metadata?: TrackMetadata;
    fingerprint?: AnalysisFingerprint;
    neteaseFilenameFormat: NeteaseFilenameFormat;
    highLevel?: HighLevelAnalysis;
    audio: DecodedAudioData;
    onProgress?: (progress: AnalysisWorkerProgress) => void;
    timeoutMs?: number;
  }) => Promise<TrackAnalysis>;
};

export const ESSENTIA_MODEL_IDS = [
  'musicnn_embedding',
  'mood_aggressive',
  'mood_happy',
  'mood_relaxed',
  'mood_party',
  'mood_sad',
  'voice_instrumental',
  'emomusic',
  'muse',
  'mirex',
  'discogs_effnet_embedding',
  'genre_discogs400',
  'discogs_mood_theme',
  'discogs_approachability',
  'discogs_instrumentation',
  'discogs_timbre',
  'discogs_danceability',
] as const;

const MSD_MUSICNN_TAGS = [
  'rock', 'pop', 'alternative', 'indie', 'electronic', 'female vocalists',
  'dance', '00s', 'alternative rock', 'jazz', 'beautiful', 'metal',
  'chillout', 'male vocalists', 'classic rock', 'soul', 'indie rock',
  'Mellow', 'electronica', '80s', 'folk', '90s', 'chill', 'instrumental',
  'punk', 'oldies', 'blues', 'hard rock', 'ambient', 'acoustic',
  'experimental', 'female vocalist', 'guitar', 'Hip-Hop', '70s', 'party',
  'country', 'easy listening', 'sexy', 'catchy', 'funk', 'electro',
  'heavy metal', 'Progressive rock', '60s', 'rnb', 'indie pop', 'sad',
  'House', 'happy',
] as const;

const BROAD_GENRE_TAGS: Record<string, readonly string[]> = {
  cla: ['classic rock', 'oldies', '60s', '70s', '80s', '90s'],
  dan: ['dance', 'electronic', 'electronica', 'electro', 'party', 'House'],
  hip: ['Hip-Hop'],
  jaz: ['jazz'],
  pop: ['pop', 'indie', 'indie pop', 'catchy'],
  rhy: ['soul', 'funk', 'rnb'],
  roc: [
    'rock', 'alternative', 'alternative rock', 'metal', 'indie rock',
    'punk', 'hard rock', 'heavy metal', 'Progressive rock',
  ],
};

export function deriveBroadGenreFromMsdTags(scores: number[]): AnalysisLabel | null {
  let best: AnalysisLabel | null = null;
  for (const [label, tags] of Object.entries(BROAD_GENRE_TAGS)) {
    const confidence = tags.reduce((maximum, tag) => {
      const index = MSD_MUSICNN_TAGS.indexOf(tag as typeof MSD_MUSICNN_TAGS[number]);
      const score = index >= 0 ? scores[index] : Number.NaN;
      return Number.isFinite(score) ? Math.max(maximum, score) : maximum;
    }, Number.NEGATIVE_INFINITY);
    if (Number.isFinite(confidence) && (!best || confidence > best.confidence)) {
      best = { label, confidence };
    }
  }
  return best;
}

/**
 * Groups the frame-wise MusiCNN mel rows into the 3D batches expected by the
 * embedding model. TensorFlow.js does not reshape a nested 2D array when a
 * 3D shape is supplied, so the batch nesting must be explicit here.
 */
export function batchMusiCnnMelRows(
  melRows: number[][],
  patchSize: number,
  melBands: number,
): number[][][] {
  const safePatchSize = Math.max(1, Math.trunc(patchSize));
  const safeMelBands = Math.max(1, Math.trunc(melBands));
  const batchCount = Math.max(1, Math.ceil(melRows.length / safePatchSize));
  const paddedRows = Array.from({ length: batchCount * safePatchSize }, (_, index) => {
    const source = melRows[index] ?? [];
    return Array.from({ length: safeMelBands }, (_, bandIndex) => source[bandIndex] ?? 0);
  });
  return Array.from({ length: batchCount }, (_, batchIndex) =>
    paddedRows.slice(batchIndex * safePatchSize, (batchIndex + 1) * safePatchSize),
  );
}

/** Build TensorFlow input from contiguous frame-major storage without a
 * second nested JavaScript array. The unused tail is already zero-filled by
 * Float32Array allocation, preserving the historical padding rule. */
export function batchMusiCnnMelBuffer(
  melBuffer: Float32Array,
  frameCount: number,
  patchSize: number,
  melBands: number,
): { values: Float32Array; batchCount: number } {
  const safeFrameCount = Math.max(0, Math.trunc(frameCount));
  const safePatchSize = Math.max(1, Math.trunc(patchSize));
  const safeMelBands = Math.max(1, Math.trunc(melBands));
  const batchCount = Math.max(1, Math.ceil(safeFrameCount / safePatchSize));
  const paddedLength = batchCount * safePatchSize * safeMelBands;
  if (melBuffer.length === paddedLength) {
    const usedLength = safeFrameCount * safeMelBands;
    let paddedTailIsZero = true;
    for (let index = usedLength; index < melBuffer.length; index += 1) {
      if (melBuffer[index] !== 0) {
        paddedTailIsZero = false;
        break;
      }
    }
    if (paddedTailIsZero) return { values: melBuffer, batchCount };
  }
  const values = new Float32Array(batchCount * safePatchSize * safeMelBands);
  const copyFrames = Math.min(safeFrameCount, Math.floor(melBuffer.length / safeMelBands));
  for (let frame = 0; frame < copyFrames; frame += 1) {
    const offset = frame * safeMelBands;
    values.set(melBuffer.subarray(offset, offset + safeMelBands), offset);
  }
  return { values, batchCount };
}

// A 64-patch MusiCNN execution can take longer than the progress watchdog in
// WebKit for a long song. Keep each graph call bounded so progress is emitted
// before the watchdog interval while the immutable per-song timeout remains
// the overall upper bound.
export const MUSICCNN_INFERENCE_BATCH_SIZE = 8;
export const MUSICCNN_CPU_INFERENCE_BATCH_SIZE = 1;

export function musicCnnInferenceBatches(
  patchCount: number,
  batchSize = MUSICCNN_INFERENCE_BATCH_SIZE,
): Array<{ offset: number; validPatches: number }> {
  const safePatchCount = Math.max(0, Math.trunc(patchCount));
  const safeBatchSize = Math.max(1, Math.trunc(batchSize));
  const batches: Array<{ offset: number; validPatches: number }> = [];
  for (let offset = 0; offset < safePatchCount; offset += safeBatchSize) {
    batches.push({
      offset,
      validPatches: Math.min(safeBatchSize, safePatchCount - offset),
    });
  }
  return batches;
}

/** Keep the MusiCNN graph input shape stable for the tail batch. The model
 * accepts a fixed bounded batch in the desktop WebKit CPU backend more
 * reliably than a dynamically smaller final batch. */
export function padMusiCnnInferenceBatch(
  melBuffer: Float32Array,
  offset: number,
  validPatches: number,
  patchStride: number,
  batchSize = MUSICCNN_INFERENCE_BATCH_SIZE,
): Float32Array {
  const safeOffset = Math.max(0, Math.trunc(offset));
  const safeBatchSize = Math.max(1, Math.trunc(batchSize));
  const safeValidPatches = Math.max(0, Math.min(
    safeBatchSize,
    Math.trunc(validPatches),
  ));
  const safePatchStride = Math.max(1, Math.trunc(patchStride));
  const values = new Float32Array(safeBatchSize * safePatchStride);
  const sourceStart = safeOffset * safePatchStride;
  const sourceEnd = Math.min(
    melBuffer.length,
    sourceStart + safeValidPatches * safePatchStride,
  );
  if (sourceEnd > sourceStart) {
    values.set(melBuffer.subarray(sourceStart, sourceEnd));
  }
  return values;
}

function discogsGenreLabels(labels: AnalysisLabel[]): AnalysisLabel[] {
  return labels
    .filter((label) => Number.isFinite(label.confidence))
    .sort((left, right) => right.confidence - left.confidence)
    .filter((label, index) => label.confidence >= 0.2 || index === 0)
    .slice(0, 5);
}

export type TrackMetadata = {
  title: string;
  artist: string;
  album: string;
  genre?: string | null;
};

export type NeteaseFilenameFormat = 'title_only' | 'artist_title' | 'title_artist';

export type AnalysisFingerprint = {
  sizeBytes: number;
  modifiedAt: number | null;
};

export const TRACK_ANALYSIS_VERSION = '0.2.0';

export type EssentiaInstance = {
  arrayToVector: (input: Float32Array) => any;
  vectorToArray: (input: any) => Float32Array;
  FrameGenerator: (audio: Float32Array, frameSize?: number, hopSize?: number) => any;
  TensorflowInputMusiCNN: (frame: any) => { bands: any };
  TensorflowInputDiscogsEffNet?: (frame: any) => { bands: any };
  MonoMixer: (left: any, right: any) => { audio: any };
  KeyExtractor: (
    audio: any,
    averageDetuningCorrection?: boolean,
    frameSize?: number,
    hopSize?: number,
    hpcpSize?: number,
    maxFrequency?: number,
    maximumSpectralPeaks?: number,
    minFrequency?: number,
    pcpThreshold?: number,
    profileType?: string,
    sampleRate?: number,
    spectralPeaksThreshold?: number,
    tuningFrequency?: number,
    weightType?: string,
    windowType?: string,
  ) => any;
  RhythmExtractor2013: (audio: any, maxTempo?: number, method?: string, minTempo?: number) => any;
  LoudnessEBUR128: (
    left: any,
    right: any,
    hopSize?: number,
    sampleRate?: number,
    startAtZero?: boolean,
  ) => any;
  BeatsLoudness: (
    audio: any,
    beatDuration?: number,
    beatWindowDuration?: number,
    beats?: number[],
    frequencyBands?: number[],
    sampleRate?: number,
  ) => any;
  algorithms?: {
    BeatsLoudness: (
      audio: any,
      beatDuration: number,
      beatWindowDuration: number,
      beats: any,
      frequencyBands: any,
      sampleRate: number,
    ) => any;
  };
  Energy: (audio: any) => any;
  Danceability: (
    audio: any,
    maxTau?: number,
    minTau?: number,
    sampleRate?: number,
    tauMultiplier?: number,
  ) => any;
  delete: () => void;
  /** Flush Embind handles whose C++ deletion is deferred by the WebView. */
  flushPendingDeletes?: () => void;
  /** Create a short-lived native instance for bounded long-song extraction. */
  createInstance?: () => EssentiaInstance;
};

export type MusicnnMelProgress = {
  processed: number;
  total: number;
};

export type MusicnnMelFeatures = {
  melRows: number[][];
  melBuffer: Float32Array;
  patchSize: number;
  melBands: number;
  frameCount: number;
};

const MUSICNN_FRAME_SIZE = 512;
const MUSICNN_HOP_SIZE = 256;
const MUSICNN_PATCH_SIZE = 187;
const MUSICNN_MEL_BANDS = 96;
const MUSICNN_PROGRESS_BATCH = 32;
const DISCOGS_EFFNET_FRAME_SIZE = 512;
const DISCOGS_EFFNET_HOP_SIZE = 256;
const DISCOGS_EFFNET_PATCH_SIZE = 128;
const DISCOGS_EFFNET_MEL_BANDS = 96;
const DISCOGS_EFFNET_BATCH_SIZE = 64;
const ESSENTIA_FRAME_INSTANCE_BATCH = 512;

// WebKit does not provide a reliable per-process CPU quota for detached
// WebContent XPC processes. Keep long-running Essentia loops cooperative: a
// short pause after each small batch keeps the hidden acceptance runner from
// monopolising a core while preserving the complete signal and model output.
// WebKit's detached content process needs a real event-loop window after
// native Embind objects are deleted; a short 40 ms pause still lets the WASM
// heap grow until the process is reclaimed under sustained frame extraction.
const ANALYSIS_YIELD_MS = 250;

function yieldToAnalysisWorker(delayMs = ANALYSIS_YIELD_MS): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, Math.max(0, delayMs)));
}

function melFilterBank(
  fftLength: number,
  sampleRate: number,
  melBands: number,
): Float32Array {
  const binCount = Math.floor(fftLength / 2) + 1;
  const hzToMel = (frequency: number) => 2595 * Math.log10(1 + frequency / 700);
  const melToHz = (mel: number) => 700 * (10 ** (mel / 2595) - 1);
  const lowerMel = hzToMel(0);
  const upperMel = hzToMel(sampleRate / 2);
  const points = Array.from({ length: melBands + 2 }, (_, index) =>
    melToHz(lowerMel + (upperMel - lowerMel) * index / (melBands + 1)));
  const bins = points.map((frequency) => Math.min(
    binCount - 1,
    Math.max(0, Math.floor((fftLength + 1) * frequency / sampleRate)),
  ));
  const filters = new Float32Array(binCount * melBands);
  for (let band = 0; band < melBands; band += 1) {
    const left = bins[band];
    const center = bins[band + 1];
    const right = bins[band + 2];
    for (let bin = left; bin < center; bin += 1) {
      if (center > left) filters[bin * melBands + band] = (bin - left) / (center - left);
    }
    for (let bin = center; bin <= right; bin += 1) {
      if (right > center) filters[bin * melBands + band] = (right - bin) / (right - center);
    }
  }
  return filters;
}

async function computeJavascriptMelRows(
  signal: Float32Array,
  onProgress: ((progress: MusicnnMelProgress) => void) | undefined,
  frameSize: number,
  hopSize: number,
  patchSize: number,
  melBands: number,
  collectRows = false,
): Promise<MusicnnMelFeatures> {
  const frameCount = signal.length >= frameSize
    ? Math.floor((signal.length - frameSize) / hopSize) + 1
    : 0;
  const melBuffer = new Float32Array(frameCount * melBands);
  const melRows: number[][] = [];
  if (frameCount === 0) {
    return { melRows, melBuffer, patchSize, melBands, frameCount };
  }

  const binCount = Math.floor(frameSize / 2) + 1;
  const filterValues = melFilterBank(frameSize, BOUNDED_ANALYSIS_SAMPLE_RATE, melBands);
  const window = new Float64Array(frameSize);
  const real = new Float64Array(frameSize);
  const imaginary = new Float64Array(frameSize);
  const power = new Float64Array(binCount);
  for (let index = 0; index < frameSize; index += 1) {
    window[index] = 0.5 - 0.5 * Math.cos(2 * Math.PI * index / frameSize);
  }
  const framesPerBatch = 128;
  for (let frameStart = 0; frameStart < frameCount; frameStart += framesPerBatch) {
    const batchEnd = Math.min(frameCount, frameStart + framesPerBatch);
    for (let frameIndex = frameStart; frameIndex < batchEnd; frameIndex += 1) {
      const sampleStart = frameIndex * hopSize;
      for (let sample = 0; sample < frameSize; sample += 1) {
        real[sample] = signal[sampleStart + sample] * window[sample];
        imaginary[sample] = 0;
      }
      for (let index = 1, reversed = 0; index < frameSize; index += 1) {
        let bit = frameSize >> 1;
        while (reversed & bit) {
          reversed ^= bit;
          bit >>= 1;
        }
        reversed ^= bit;
        if (index < reversed) {
          const realValue = real[index];
          real[index] = real[reversed];
          real[reversed] = realValue;
          const imaginaryValue = imaginary[index];
          imaginary[index] = imaginary[reversed];
          imaginary[reversed] = imaginaryValue;
        }
      }
      for (let length = 2; length <= frameSize; length <<= 1) {
        const halfLength = length >> 1;
        const angle = -2 * Math.PI / length;
        const stepReal = Math.cos(angle);
        const stepImaginary = Math.sin(angle);
        for (let start = 0; start < frameSize; start += length) {
          let weightReal = 1;
          let weightImaginary = 0;
          for (let offset = 0; offset < halfLength; offset += 1) {
            const even = start + offset;
            const odd = even + halfLength;
            const oddReal = real[odd] * weightReal - imaginary[odd] * weightImaginary;
            const oddImaginary = real[odd] * weightImaginary + imaginary[odd] * weightReal;
            const evenReal = real[even];
            const evenImaginary = imaginary[even];
            real[even] = evenReal + oddReal;
            imaginary[even] = evenImaginary + oddImaginary;
            real[odd] = evenReal - oddReal;
            imaginary[odd] = evenImaginary - oddImaginary;
            const nextWeightReal = weightReal * stepReal - weightImaginary * stepImaginary;
            weightImaginary = weightReal * stepImaginary + weightImaginary * stepReal;
            weightReal = nextWeightReal;
          }
        }
      }
      for (let bin = 0; bin < binCount; bin += 1) {
        power[bin] = real[bin] * real[bin] + imaginary[bin] * imaginary[bin];
      }
      const targetOffset = frameIndex * melBands;
      for (let band = 0; band < melBands; band += 1) {
        let sum = 0;
        for (let bin = 0; bin < binCount; bin += 1) {
          sum += power[bin] * filterValues[bin * melBands + band];
        }
        const value = Math.log10(1 + 10_000 * Math.max(0, sum));
        melBuffer[targetOffset + band] = Number.isFinite(value) ? value : 0;
      }
      if (collectRows) {
        melRows.push(Array.from(melBuffer.subarray(targetOffset, targetOffset + melBands)));
      }
    }
    onProgress?.({ processed: batchEnd, total: frameCount });
    if (batchEnd < frameCount) await yieldToAnalysisWorker();
  }
  return { melRows, melBuffer, patchSize, melBands, frameCount };
}

/**
 * Extract MusiCNN rows without using EssentiaTFInputExtractor.computeFrameWise.
 * The published wrapper leaves the per-frame Embind vectors alive until the
 * whole extractor is deleted, which makes sequential long-song analysis grow
 * the WASM heap.  Copy each row before releasing its native vectors instead.
 */
export async function computeMusiCnnMelRows(
  essentia: EssentiaInstance,
  signal: Float32Array,
  onProgress?: (progress: MusicnnMelProgress) => void,
  options: { collectRows?: boolean; tensorRuntime?: any } = {},
): Promise<MusicnnMelFeatures> {
  const collectRows = options.collectRows ?? true;
  if (options.tensorRuntime?.signal?.stft) {
    return computeJavascriptMelRows(
      signal,
      onProgress,
      MUSICNN_FRAME_SIZE,
      MUSICNN_HOP_SIZE,
      MUSICNN_PATCH_SIZE,
      MUSICNN_MEL_BANDS,
      collectRows,
    );
  }
  const melRows: number[][] = [];
  // FrameGenerator materializes every frame in a native vector. The bundled
  // runtime therefore creates one 512-sample vector per frame, copies its
  // output, and releases it before the next frame. Lightweight test doubles
  // without a runtime factory keep the container-based fallback.
  const expectedTotal = signal.length >= MUSICNN_FRAME_SIZE
    ? Math.floor((signal.length - MUSICNN_FRAME_SIZE) / MUSICNN_HOP_SIZE) + 1
    : 0;
  let total = expectedTotal;
  let melBuffer = new Float32Array(total * MUSICNN_MEL_BANDS);

  const appendFrame = (
    frameEssentia: EssentiaInstance,
    frame: any,
    index: number,
    releaseFrame = true,
  ) => {
    try {
      const output = frameEssentia.TensorflowInputMusiCNN(frame);
      const bands = output?.bands;
      try {
        const offset = index * MUSICNN_MEL_BANDS;
        const valueCount = copyEssentiaVectorValues(
          frameEssentia,
          bands,
          melBuffer,
          offset,
          MUSICNN_MEL_BANDS,
        );
        if (collectRows) {
          melRows.push(Array.from(melBuffer.subarray(offset, offset + valueCount)));
        }
      } finally {
        releaseVector(bands);
        releaseVector(output);
      }
    } finally {
      if (releaseFrame) releaseVector(frame);
    }
  };

  if (essentia.createInstance && expectedTotal > 0) {
    // FrameGenerator materializes only one bounded chunk at a time. The
    // bundled Essentia frontend is considerably more stable with that
    // contract than with one arrayToVector call per frame in WebKit.
    for (let chunkStart = 0; chunkStart < expectedTotal; chunkStart += ESSENTIA_FRAME_INSTANCE_BATCH) {
      const frameCount = Math.min(
        ESSENTIA_FRAME_INSTANCE_BATCH,
        expectedTotal - chunkStart,
      );
      const signalStart = chunkStart * MUSICNN_HOP_SIZE;
      const signalEnd = Math.min(
        signal.length,
        signalStart + (frameCount - 1) * MUSICNN_HOP_SIZE + MUSICNN_FRAME_SIZE,
      );
      const frameEssentia = essentia.createInstance?.() ?? essentia;
      let frames: any = null;
      try {
        frames = frameEssentia.FrameGenerator(
          signal.subarray(signalStart, signalEnd),
          MUSICNN_FRAME_SIZE,
          MUSICNN_HOP_SIZE,
        );
        const actualFrameCount = Math.min(
          frameCount,
          Math.max(0, Number(frames?.size?.() ?? 0)),
        );
        for (let localIndex = 0; localIndex < actualFrameCount; localIndex += 1) {
          const index = chunkStart + localIndex;
          appendFrame(frameEssentia, frames.get(localIndex), index);
          frameEssentia.flushPendingDeletes?.();
          const processed = index + 1;
          if (processed === total || processed % MUSICNN_PROGRESS_BATCH === 0) {
            onProgress?.({ processed, total });
          }
        }
      } finally {
        releaseVector(frames);
        frameEssentia.flushPendingDeletes?.();
        if (frameEssentia !== essentia) {
          frameEssentia.delete();
          frameEssentia.flushPendingDeletes?.();
        }
      }
      if (chunkStart + frameCount < total) {
        await yieldToAnalysisWorker();
      }
    }
  } else {
    const frames = essentia.FrameGenerator(signal, MUSICNN_FRAME_SIZE, MUSICNN_HOP_SIZE);
    try {
      total = Math.max(0, Number(frames?.size?.() ?? 0));
      melBuffer = new Float32Array(total * MUSICNN_MEL_BANDS);
      for (let index = 0; index < total; index += 1) {
        try {
          appendFrame(essentia, frames.get(index), index);
        } finally {
          essentia.flushPendingDeletes?.();
        }
        const processed = index + 1;
        if (processed === total || processed % MUSICNN_PROGRESS_BATCH === 0) {
          onProgress?.({ processed, total });
          if (processed < total) await yieldToAnalysisWorker();
        }
      }
    } finally {
      releaseVector(frames);
    }
  }

  return {
    melRows,
    melBuffer,
    patchSize: MUSICNN_PATCH_SIZE,
    melBands: MUSICNN_MEL_BANDS,
    frameCount: total,
  };
}

/** Stream the official Discogs-EffNet [N,128,96] batches. Only one 64-patch
 * batch plus the current 128-frame patch is retained in memory. */
export async function* streamDiscogsEffnetMelBatches(
  essentia: EssentiaInstance,
  signal: Float32Array,
  onProgress?: (progress: DiscogsEffnetMelProgress) => void,
  options: { tensorRuntime?: any } = {},
): AsyncGenerator<DiscogsEffnetMelBatch> {
  if (options.tensorRuntime?.signal?.stft) {
    const features = await computeJavascriptMelRows(
      signal,
      ({ processed, total }) => onProgress?.({
        processedPatches: Math.max(1, Math.ceil(processed / DISCOGS_EFFNET_PATCH_SIZE)),
        totalPatches: Math.max(1, Math.ceil(total / DISCOGS_EFFNET_PATCH_SIZE)),
      }),
      DISCOGS_EFFNET_FRAME_SIZE,
      DISCOGS_EFFNET_HOP_SIZE,
      DISCOGS_EFFNET_PATCH_SIZE,
      DISCOGS_EFFNET_MEL_BANDS,
    );
    const totalPatches = Math.max(1, Math.ceil(features.frameCount / DISCOGS_EFFNET_PATCH_SIZE));
    const { values } = batchMusiCnnMelBuffer(
      features.melBuffer,
      features.frameCount,
      DISCOGS_EFFNET_PATCH_SIZE,
      DISCOGS_EFFNET_MEL_BANDS,
    );
    for (let patchStart = 0; patchStart < totalPatches; patchStart += DISCOGS_EFFNET_BATCH_SIZE) {
      const validPatches = Math.min(DISCOGS_EFFNET_BATCH_SIZE, totalPatches - patchStart);
      const batchOffset = patchStart
        * DISCOGS_EFFNET_PATCH_SIZE
        * DISCOGS_EFFNET_MEL_BANDS;
      const batchLength = DISCOGS_EFFNET_BATCH_SIZE
        * DISCOGS_EFFNET_PATCH_SIZE
        * DISCOGS_EFFNET_MEL_BANDS;
      // The final group can contain fewer than 64 valid patches, but the
      // bs64 graph still requires a complete [64,128,96] tensor. Copy into a
      // zero-filled fixed-size buffer instead of yielding a short slice.
      const batchValues = new Float32Array(batchLength);
      batchValues.set(values.subarray(
        batchOffset,
        Math.min(values.length, batchOffset + batchLength),
      ));
      onProgress?.({
        processedPatches: Math.min(totalPatches, patchStart + validPatches),
        totalPatches,
      });
      yield {
        values: batchValues,
        batchSize: DISCOGS_EFFNET_BATCH_SIZE,
        framesPerPatch: DISCOGS_EFFNET_PATCH_SIZE,
        melBands: DISCOGS_EFFNET_MEL_BANDS,
        validPatches,
      };
      if (patchStart + validPatches < totalPatches) await yieldToAnalysisWorker();
    }
    return;
  }
  const patchRows: number[][] = [];
  let batchValues = new Float32Array(
    DISCOGS_EFFNET_BATCH_SIZE * DISCOGS_EFFNET_PATCH_SIZE * DISCOGS_EFFNET_MEL_BANDS,
  );
  let validPatches = 0;
  let processedPatches = 0;
  if (!essentia.TensorflowInputDiscogsEffNet) {
    throw new Error('Essentia.js 未提供 Discogs-EffNet Mel 前端');
  }

  const expectedTotalFrames = signal.length >= DISCOGS_EFFNET_FRAME_SIZE
    ? Math.floor((signal.length - DISCOGS_EFFNET_FRAME_SIZE) / DISCOGS_EFFNET_HOP_SIZE) + 1
    : 0;
  let totalFrames = expectedTotalFrames;
  let totalPatches = Math.max(1, Math.ceil(totalFrames / DISCOGS_EFFNET_PATCH_SIZE));

  const appendFrame = (frameEssentia: EssentiaInstance, frame: any, index: number): void => {
    const extract = frameEssentia.TensorflowInputDiscogsEffNet;
    if (!extract) throw new Error('Essentia.js 未提供 Discogs-EffNet Mel 前端');
    const output = extract(frame);
    const bands = output?.bands;
    try {
      const row = new Float32Array(DISCOGS_EFFNET_MEL_BANDS);
      copyEssentiaVectorValues(frameEssentia, bands, row, 0, DISCOGS_EFFNET_MEL_BANDS);
      patchRows.push(Array.from(row));
      // Keep the global frame index in the signature so the bounded and
      // fallback paths share identical patch ordering.
      void index;
    } finally {
      releaseVector(bands);
      releaseVector(output);
    }
  };

  const finishPatch = (): void => {
    const outputOffset = validPatches
      * DISCOGS_EFFNET_PATCH_SIZE
      * DISCOGS_EFFNET_MEL_BANDS;
    for (let frameIndex = 0; frameIndex < DISCOGS_EFFNET_PATCH_SIZE; frameIndex += 1) {
      const source = patchRows[frameIndex];
      const rowOffset = outputOffset + frameIndex * DISCOGS_EFFNET_MEL_BANDS;
      for (let band = 0; band < DISCOGS_EFFNET_MEL_BANDS; band += 1) {
        batchValues[rowOffset + band] = source?.[band] ?? 0;
      }
    }
    patchRows.length = 0;
    validPatches += 1;
    processedPatches += 1;
  };

  const takeBatch = (): DiscogsEffnetMelBatch | null => {
    if (validPatches < DISCOGS_EFFNET_BATCH_SIZE && processedPatches < totalPatches) {
      return null;
    }
    const batch: DiscogsEffnetMelBatch = {
      values: batchValues,
      batchSize: DISCOGS_EFFNET_BATCH_SIZE,
      framesPerPatch: DISCOGS_EFFNET_PATCH_SIZE,
      melBands: DISCOGS_EFFNET_MEL_BANDS,
      validPatches,
    };
    batchValues = new Float32Array(
      DISCOGS_EFFNET_BATCH_SIZE * DISCOGS_EFFNET_PATCH_SIZE * DISCOGS_EFFNET_MEL_BANDS,
    );
    validPatches = 0;
    return batch;
  };

  const consumeFrame = async function* (
    frameEssentia: EssentiaInstance,
    frameStart: number,
    frameCount: number,
    getFrame: (localIndex: number) => any,
    releaseEachFrame = true,
  ): AsyncGenerator<DiscogsEffnetMelBatch> {
    for (let localIndex = 0; localIndex < frameCount; localIndex += 1) {
      const index = frameStart + localIndex;
      const frame = getFrame(localIndex);
      try {
        appendFrame(frameEssentia, frame, index);
      } finally {
        if (releaseEachFrame) releaseVector(frame);
        frameEssentia.flushPendingDeletes?.();
      }
      const patchComplete = patchRows.length === DISCOGS_EFFNET_PATCH_SIZE
        || index + 1 === totalFrames;
      if (patchComplete) {
        finishPatch();
        const batch = takeBatch();
        if (batch) {
          onProgress?.({ processedPatches, totalPatches });
          yield batch;
          if (processedPatches < totalPatches) {
            await yieldToAnalysisWorker();
          }
        }
      }
      if ((index + 1) % 32 === 0 && !patchComplete) {
        await yieldToAnalysisWorker();
      }
    }
  };

  if (essentia.createInstance && expectedTotalFrames > 0) {
    for (let chunkStart = 0; chunkStart < expectedTotalFrames; chunkStart += ESSENTIA_FRAME_INSTANCE_BATCH) {
      const frameCount = Math.min(
        ESSENTIA_FRAME_INSTANCE_BATCH,
        expectedTotalFrames - chunkStart,
      );
      const signalStart = chunkStart * DISCOGS_EFFNET_HOP_SIZE;
      const signalEnd = Math.min(
        signal.length,
        signalStart + (frameCount - 1) * DISCOGS_EFFNET_HOP_SIZE + DISCOGS_EFFNET_FRAME_SIZE,
      );
      const frameEssentia = essentia.createInstance?.() ?? essentia;
      let frames: any = null;
      try {
        frames = frameEssentia.FrameGenerator(
          signal.subarray(signalStart, signalEnd),
          DISCOGS_EFFNET_FRAME_SIZE,
          DISCOGS_EFFNET_HOP_SIZE,
        );
        const actualFrameCount = Math.min(
          frameCount,
          Math.max(0, Number(frames?.size?.() ?? 0)),
        );
        yield* consumeFrame(
          frameEssentia,
          chunkStart,
          actualFrameCount,
          (localIndex) => frames.get(localIndex),
        );
      } finally {
        releaseVector(frames);
        frameEssentia.flushPendingDeletes?.();
        if (frameEssentia !== essentia) {
          frameEssentia.delete();
          frameEssentia.flushPendingDeletes?.();
        }
      }
      if (chunkStart + frameCount < totalFrames) {
        await yieldToAnalysisWorker();
      }
    }
  } else {
    // Lightweight test doubles and older wrappers may not expose an instance
    // factory. Preserve their behavior, while the bundled runtime always uses
    // the bounded branch above.
    const frames = essentia.FrameGenerator(
      signal,
      DISCOGS_EFFNET_FRAME_SIZE,
      DISCOGS_EFFNET_HOP_SIZE,
    );
    try {
      totalFrames = Math.max(0, Number(frames?.size?.() ?? 0));
      totalPatches = Math.max(1, Math.ceil(totalFrames / DISCOGS_EFFNET_PATCH_SIZE));
      yield* consumeFrame(essentia, 0, totalFrames, (index) => frames.get(index));
    } finally {
      releaseVector(frames);
    }
  }

  if (totalFrames === 0) {
    if (validPatches === 0) {
      onProgress?.({ processedPatches: 1, totalPatches: 1 });
      yield {
        values: batchValues,
        batchSize: DISCOGS_EFFNET_BATCH_SIZE,
        framesPerPatch: DISCOGS_EFFNET_PATCH_SIZE,
        melBands: DISCOGS_EFFNET_MEL_BANDS,
        validPatches: 1,
      };
    }
  } else if (validPatches > 0) {
    // The final patch is padded by the zero-filled batch buffer.
    const batch = takeBatch();
    if (batch) {
      onProgress?.({ processedPatches, totalPatches });
      yield batch;
    }
  }
}

/** Compatibility wrapper used by tests and callers that explicitly need all
 * batches. Production analysis consumes the async stream above. */
export async function computeDiscogsEffnetMelBatches(
  essentia: EssentiaInstance,
  signal: Float32Array,
  onProgress?: (progress: DiscogsEffnetMelProgress) => void,
): Promise<DiscogsEffnetMelBatch[]> {
  const batches: DiscogsEffnetMelBatch[] = [];
  for await (const batch of streamDiscogsEffnetMelBatches(essentia, signal, onProgress)) {
    batches.push(batch);
  }
  return batches;
}

const DROP_BEAT_COUNT = 32;

export function selectDropBeatWindow(
  beatPositions: number[],
  beatLoudness: number[],
  durationSeconds: number,
  beatCount = DROP_BEAT_COUNT,
): { startIndex: number; endIndex: number; averageLoudness: number } | null {
  if (!Number.isFinite(durationSeconds) || durationSeconds <= 0 || beatCount <= 0) {
    return null;
  }
  const paired = beatPositions
    .map((position, index) => ({
      position,
      loudness: beatLoudness[index] ?? Number.NaN,
      sourceIndex: index,
    }))
    .filter(({ position, loudness }) => Number.isFinite(position) && Number.isFinite(loudness));
  if (paired.length < beatCount) {
    return null;
  }

  const eligible = paired.filter(({ position }) => position >= durationSeconds * 0.15
    && position <= durationSeconds * 0.85);
  if (eligible.length < beatCount) {
    return null;
  }

  let best: { startIndex: number; endIndex: number; averageLoudness: number } | null = null;
  for (let offset = 0; offset <= eligible.length - beatCount; offset += 1) {
    const window = eligible.slice(offset, offset + beatCount);
    const average = window.reduce((sum, beat) => sum + beat.loudness, 0) / beatCount;
    if (!best || average > best.averageLoudness) {
      best = {
        startIndex: window[0].sourceIndex,
        endIndex: window[window.length - 1].sourceIndex,
        averageLoudness: average,
      };
    }
  }
  return best;
}

const NEGATIVE_HIGH_LEVEL_LABELS = new Set([
  'non_aggressive',
  'non_happy',
  'non_relaxed',
  'non_party',
  'non_sad',
]);

export function filterHighLevelLabels(
  labels: AnalysisLabel[],
  threshold = 0.75,
): { accepted: AnalysisLabel[]; filtered: Array<{ label: string; confidence: number | null; reason: string }> } {
  const accepted: AnalysisLabel[] = [];
  const filtered: Array<{ label: string; confidence: number | null; reason: string }> = [];
  for (const label of labels) {
    const normalized = label.label.trim().toLowerCase();
    const confidence = Number.isFinite(label.confidence) ? label.confidence : null;
    if (NEGATIVE_HIGH_LEVEL_LABELS.has(normalized)) {
      filtered.push({ label: label.label, confidence, reason: 'negative_label' });
    } else if (!Number.isFinite(label.confidence) || label.confidence < threshold) {
      filtered.push({ label: label.label, confidence, reason: 'below_threshold' });
    } else {
      accepted.push(label);
    }
  }
  return { accepted, filtered };
}

type EssentiaConstructor = new (wasm: any, debug?: boolean) => EssentiaInstance;

type EssentiaRuntime = {
  essentia: EssentiaInstance;
  wasmModule: any;
};

let essentiaRuntimePromise: Promise<EssentiaRuntime> | null = null;

async function getEssentiaRuntime(): Promise<EssentiaRuntime> {
  if (!essentiaRuntimePromise) {
    essentiaRuntimePromise = Promise.all([
      import('essentia.js/dist/essentia-wasm.es.js'),
      import('essentia.js/dist/essentia.js-extractor.es.js'),
    ]).then(([wasmModule, extractorModule]) => {
      const Constructor = extractorModule.default as unknown as EssentiaConstructor;
      const wasmBackend = wasmModule.EssentiaWASM as {
        flushPendingDeletes?: () => void;
      };
      const pendingDeleteFlusher = wasmBackend.flushPendingDeletes;
      const createInstance = (): EssentiaInstance => {
        const instance = new Constructor(wasmBackend, false);
        // Every bounded instance must be able to create the next bounded
        // instance too.  The runtime is rebuilt between MusiCNN and Discogs;
        // without this assignment the rebuilt instance silently falls back to
        // one FrameGenerator for the whole song.
        instance.createInstance = createInstance;
        if (!instance.TensorflowInputDiscogsEffNet) {
          instance.TensorflowInputDiscogsEffNet = (frame: any) =>
            instance.TensorflowInputMusiCNN(frame);
        }
        if (typeof pendingDeleteFlusher === 'function') {
          instance.flushPendingDeletes = () => pendingDeleteFlusher();
        }
        return instance;
      };
      const essentia = createInstance();
      // Essentia.js 0.1.3 does not expose the newer Discogs-specific alias;
      // install a compatibility alias on each runtime instance. The Discogs frontend
      // itself only calls TensorflowInputDiscogsEffNet, keeping its 128-frame
      // contract separate from the legacy MusiCNN 187-frame path while newer
      // Essentia builds can provide their native primitive.
      return {
        essentia,
        wasmModule,
      };
    });
  }
  return essentiaRuntimePromise;
}

async function getEssentia(): Promise<EssentiaInstance> {
  return (await getEssentiaRuntime()).essentia;
}

function resetEssentiaRuntimeInstance(runtime: EssentiaRuntime): EssentiaInstance {
  const previous = runtime.essentia;
  const createInstance = previous.createInstance;
  if (!createInstance) {
    return previous;
  }
  previous.flushPendingDeletes?.();
  previous.delete();
  previous.flushPendingDeletes?.();
  const next = createInstance();
  runtime.essentia = next;
  return next;
}

function finiteNumber(value: unknown): number | null {
  const number = typeof value === 'number' ? value : Number(value);
  return Number.isFinite(number) ? number : null;
}

function vectorToNumbers(essentia: EssentiaInstance, value: any): number[] {
  if (!value) {
    return [];
  }
  if (Array.isArray(value) || ArrayBuffer.isView(value)) {
    return Array.from(value as ArrayLike<number>).filter((item) => Number.isFinite(item));
  }
  try {
    return Array.from(essentia.vectorToArray(value)).filter((item) => Number.isFinite(item));
  } catch (error) {
    if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
    return [];
  }
}

function releaseVector(value: any): void {
  if (value && typeof value.delete === 'function') {
    value.delete();
  }
}

function copyEssentiaVectorValues(
  essentia: EssentiaInstance,
  value: any,
  target: Float32Array,
  offset: number,
  maxLength: number,
): number {
  if (value && typeof value.size === 'function' && typeof value.get === 'function') {
    const length = Math.min(maxLength, Math.max(0, Number(value.size())));
    for (let index = 0; index < length; index += 1) {
      const number = Number(value.get(index));
      target[offset + index] = Number.isFinite(number) ? number : 0;
    }
    return length;
  }
  const values = essentia.vectorToArray(value);
  const length = Math.min(values.length, maxLength);
  for (let index = 0; index < length; index += 1) {
    const number = Number(values[index]);
    target[offset + index] = Number.isFinite(number) ? number : 0;
  }
  return length;
}

type TensorflowRuntime = {
  tf: any;
  InputExtractor: new (wasm: any, extractorType?: string, debug?: boolean) => {
    downsampleAudioBuffer: (buffer: AudioBuffer) => Promise<Float32Array>;
    computeFrameWise: (signal: Float32Array, hopSize?: number) => any;
    delete: () => void;
  };
};

let tensorflowRuntimePromise: Promise<TensorflowRuntime> | null = null;

async function getTensorflowRuntime(): Promise<TensorflowRuntime> {
  if (!tensorflowRuntimePromise) {
    // These dynamic imports are split into local Vite chunks and bundled with
    // the desktop app. They never resolve TensorFlow.js or Essentia.js from a
    // CDN at runtime.
    tensorflowRuntimePromise = Promise.all([
      import('@tensorflow/tfjs'),
      import('essentia.js/dist/essentia.js-model.es.js'),
    ]).then(([tf, modelModule]) => ({
      tf,
      InputExtractor: modelModule.EssentiaTFInputExtractor as TensorflowRuntime['InputExtractor'],
    }));
  }
  return tensorflowRuntimePromise;
}

function modelArtifacts(model: EssentiaModelFile): { modelTopology: unknown; weightSpecs: unknown[] } {
  const parsed = JSON.parse(model.modelJson) as {
    modelTopology?: unknown;
    weightsManifest?: Array<{ weights?: unknown[] }>;
  };
  if (!parsed.modelTopology || !parsed.weightsManifest) {
    throw new Error(`Essentia 模型 ${model.id} 的结构不完整`);
  }
  return {
    modelTopology: parsed.modelTopology,
    weightSpecs: parsed.weightsManifest.flatMap((manifest) => manifest.weights ?? []),
  };
}

export async function loadTensorflowModel(tf: any, model: EssentiaModelFile): Promise<any> {
  const artifacts = modelArtifacts(model);
  const weightData = modelWeightDataBuffer(model.weightData);
  return tf.loadGraphModel(tf.io.fromMemory({
    modelTopology: artifacts.modelTopology,
    weightSpecs: artifacts.weightSpecs,
    weightData,
  }));
}

function averagePredictions(predictions: unknown, classCount: number): number[] {
  const rows = Array.isArray(predictions) ? predictions : [];
  const normalizedRows = rows.length > 0 && Array.isArray(rows[0]) ? rows : [rows];
  const totals = Array.from({ length: classCount }, () => 0);
  let count = 0;
  for (const row of normalizedRows) {
    if (!Array.isArray(row)) {
      continue;
    }
    const values = row.map((value) => Number(value));
    if (values.length < classCount || values.some((value) => !Number.isFinite(value))) {
      continue;
    }
    values.slice(0, classCount).forEach((value, index) => {
      totals[index] += value;
    });
    count += 1;
  }
  return count > 0 ? totals.map((value) => value / count) : [];
}

export function executeEssentiaModel(
  tf: any,
  model: any,
  featureTensor: any,
  outputName?: string | string[],
): any {
  // Essentia's TensorflowMusiCNN wrapper puts optional inputs before the
  // feature tensor: [isTraining, features] for the embedding model. Keep
  // that ordering here instead of relying on the topology's node order.
  const inputCount = model?.executor?.inputs?.length ?? model?.inputs?.length ?? 1;
  const run = () => {
    const inputs = inputCount === 2
      ? [tf.tensor([0], [1], 'bool'), featureTensor]
      : [featureTensor];
    try {
      return outputName ? model.execute(inputs, outputName) : model.execute(inputs);
    } finally {
      if (inputs[0] !== featureTensor) {
        inputs[0].dispose();
      }
    }
  };
  // GraphModel.execute creates a graph of intermediate tensors.  WebKit's
  // CPU backend does not reliably reclaim those intermediates between calls
  // unless the execution is enclosed in a TensorFlow scope.  Keep the
  // returned output tensor(s) alive for the caller, while disposing every
  // temporary tensor as soon as this one inference returns.
  return typeof tf.tidy === 'function' ? tf.tidy(run) : run();
}

/**
 * Native WebKit needs an explicit stable backend selection. Its remote WebGL
 * context can block forever in synchronous shader queries inside a Worker,
 * which also prevents the analysis timeout from being observed. Keep WebKit
 * on the TensorFlow.js WASM backend, which is the native CPU path; Chromium
 * retains its own backend selection. The helper is
 * deliberately injectable for deterministic tests and future WebView changes.
 */
export function shouldUseCpuTensorflowBackend(userAgent: string): boolean {
  return /AppleWebKit/i.test(userAgent)
    && !/(Chrome|Chromium|CriOS|Edg|OPR)/i.test(userAgent);
}

export async function configureTensorflowBackend(
  tf: any,
  userAgent = typeof navigator === 'undefined' ? '' : navigator.userAgent,
  preferredBackend?: 'cpu' | 'webgl' | 'wasm',
): Promise<string | undefined> {
  if (typeof tf.setBackend === 'function') {
    const backend = preferredBackend
      ?? (shouldUseCpuTensorflowBackend(userAgent) ? 'wasm' : undefined);
    if (backend) {
      if (backend === 'wasm') {
        const wasm = await import('@tensorflow/tfjs-backend-wasm');
        wasm.setWasmPaths({
          'tfjs-backend-wasm.wasm': wasmBinaryUrl,
          'tfjs-backend-wasm-simd.wasm': wasmSimdBinaryUrl,
          'tfjs-backend-wasm-threaded-simd.wasm': wasmThreadedSimdBinaryUrl,
        });
      }
      const selected = await tf.setBackend(backend);
      if (selected === false) {
        throw new Error(`无法启用 TensorFlow.js ${backend} 后端`);
      }
    }
  }
  if (typeof tf.ready === 'function') {
    await tf.ready();
  }
  return typeof tf.getBackend === 'function' ? tf.getBackend() : undefined;
}

async function runHighLevelAnalysis(
  audio: DecodedAudioData,
  models: EssentiaModelFile[],
  onProgress?: (progress: AnalysisWorkerProgress) => void,
  preferredBackend?: 'cpu' | 'webgl' | 'wasm',
): Promise<HighLevelAnalysis> {
  const modelById = new Map(models.map((model) => [model.id, model]));
  const embedding = modelById.get('musicnn_embedding');
  if (!embedding) {
    return { status: 'model_missing', reason: '未下载 Essentia MusiCNN 特征模型' };
  }

  const { tf } = await getTensorflowRuntime();
  await configureTensorflowBackend(tf, undefined, preferredBackend);
  const runtime = await getEssentiaRuntime();
  // Basic analysis and the high-level models use the same cached Essentia
  // runtime. Start MusiCNN from a fresh registry so native allocations left
  // by chunked/basic analysis cannot inflate the model phase's peak heap.
  let analysisEssentia = resetEssentiaRuntimeInstance(runtime);
  const emitProgress = (progress: AnalysisWorkerProgress) => {
    const memory = typeof tf.memory === 'function' ? tf.memory() : undefined;
    onProgress?.({
      ...progress,
      backend: typeof tf.getBackend === 'function' ? tf.getBackend() : undefined,
      tfMemory: memory
        ? {
          numTensors: memory.numTensors,
          numBytes: memory.numBytes,
          unreliable: memory.unreliable,
        }
        : undefined,
    });
  };
  const classifierModels: any[] = [];
  let embeddingModel: any = null;
  try {
    const signal = audio.musicnnSignal;
    if (!signal) {
      throw new Error('MusiCNN 输入音频准备失败');
    }
    const features = await computeMusiCnnMelRows(
      analysisEssentia,
      signal,
      ({ processed, total }) => emitProgress({
        stage: 'extractingMusiCnn',
        message: `正在提取 MusiCNN 特征 ${processed}/${total}`,
        processed,
        total,
      }),
      { collectRows: false, tensorRuntime: tf },
    );
    // The JavaScript Mel frontend is intentionally run before loading the
    // large MusiCNN graph. Keeping the graph resident while transforming a
    // long signal pushes WebKit into multi-gigabyte GC pressure before model
    // inference even begins.
    embeddingModel = await loadTensorflowModel(tf, embedding);
    classifierModels.push(embeddingModel);
    const patchSize = features.patchSize;
    const melBands = features.melBands;
    const activeBackend = typeof tf.getBackend === 'function' ? tf.getBackend() : undefined;
    const inferenceBatchSize = activeBackend === 'cpu'
      ? MUSICCNN_CPU_INFERENCE_BATCH_SIZE
      : MUSICCNN_INFERENCE_BATCH_SIZE;
    const { values: paddedMel, batchCount } = batchMusiCnnMelBuffer(
      features.melBuffer,
      features.frameCount,
      patchSize,
      melBands,
    );
    const embeddingRowsFromMusicnn: number[][] = [];
    const tagSums = new Float64Array(MSD_MUSICNN_TAGS.length);
    let tagCount = 0;
    const patchStride = patchSize * melBands;
    for (const { offset, validPatches } of musicCnnInferenceBatches(batchCount, inferenceBatchSize)) {
      const inputValues = padMusiCnnInferenceBatch(
        paddedMel,
        offset,
        validPatches,
        patchStride,
        inferenceBatchSize,
      );
      const input = tf.tensor3d(
        inputValues,
        [inferenceBatchSize, patchSize, melBands],
        'float32',
      );
      let tensor: any = null;
      let tagTensor: any = null;
      try {
        const output = executeEssentiaModel(
          tf,
          embeddingModel,
          input,
          ['model/dense/Relu', 'model/Sigmoid'],
        );
        const outputs = Array.isArray(output) ? output : [output];
        tensor = outputs[0];
        tagTensor = outputs[1];
        if (!tensor || !tagTensor) {
          throw new Error('MusiCNN 模型未同时返回 embedding 和标签输出');
        }
        const [embeddingRows, tagRows] = await Promise.all([
          tensor.array(),
          tagTensor.array(),
        ]);
        if (Array.isArray(embeddingRows)) {
          for (const row of embeddingRows.slice(0, validPatches)) {
            if (Array.isArray(row)) embeddingRowsFromMusicnn.push(row.map(Number));
          }
        }
        if (Array.isArray(tagRows)) {
          for (const row of tagRows.slice(0, validPatches)) {
            if (!Array.isArray(row)) continue;
            const values = row.slice(0, MSD_MUSICNN_TAGS.length).map(Number);
            if (values.length !== MSD_MUSICNN_TAGS.length || values.some((value) => !Number.isFinite(value))) continue;
            values.forEach((value, index) => { tagSums[index] += value; });
            tagCount += 1;
          }
        }
      } finally {
        input.dispose();
        tensor?.dispose?.();
        tagTensor?.dispose?.();
      }
      emitProgress({
        stage: 'runningMusiCnn',
        message: `正在运行 MusiCNN 模型 ${Math.min(batchCount, offset + validPatches)}/${batchCount}`,
        processed: Math.min(batchCount, offset + validPatches),
        total: batchCount,
        modelId: embedding.id,
        modelFamily: 'musicnn',
        patchCount: batchCount,
      });
      if (offset + validPatches < batchCount) {
        await yieldToAnalysisWorker();
      }
    }

    // The MusiCNN graph is no longer needed once its embedding rows and tag
    // aggregates have been copied. Release its weights before loading the
    // Discogs-EffNet graph so WebKit does not keep both large graphs resident
    // while the native Essentia heap is still occupied by the current track.
    embeddingModel?.dispose?.();
    embeddingModel = null;
    classifierModels.length = 0;
    emitProgress({
      stage: 'releasedMusiCnn',
      message: 'MusiCNN 模型资源已释放',
      modelId: embedding.id,
      modelFamily: 'musicnn',
    });

    // MusiCNN's native frontend can grow the WASM allocator even after its
    // per-frame vectors are deleted. Recreate the singleton before Discogs so
    // the next fixed-size embedding batch does not inherit that high-water
    // mark or any frontend-owned native state.
    analysisEssentia = resetEssentiaRuntimeInstance(runtime);

    if (embeddingRowsFromMusicnn.length === 0 || tagCount === 0) {
      throw new Error('MusiCNN 未返回有效的 embedding 或标签输出');
    }

    const filtered: Array<{ label: string; confidence: number | null; reason: string }> = [];
    const genre: AnalysisLabel[] = [];
    const style: AnalysisLabel[] = [];
    const mood: AnalysisLabel[] = [];
    const instrument: AnalysisLabel[] = [];
    let discogsEffnet: DiscogsEffnetAnalysis | undefined;
    const tagScores = tagCount > 0
      ? Array.from(tagSums, (value) => value / tagCount)
      : [];
    const styleResult = filterHighLevelLabels(MSD_MUSICNN_TAGS.map((label, index) => ({
      label,
      confidence: tagScores[index] ?? Number.NaN,
    })));
    style.push(...styleResult.accepted);
    filtered.push(...styleResult.filtered);
    // MusiCNN's legacy broad-genre projection is retained as an exported
    // migration helper, but it must not populate the new Discogs `genre`
    // field. The 50-tag output belongs to `style`; when the Discogs pair is
    // unavailable, the formal Genre field stays empty rather than borrowing
    // a Style label. Style labels and Discogs Genre labels are kept separate;
    // neither model is allowed to masquerade as the other field.
    const embeddingRows = embeddingRowsFromMusicnn;
    const discogsEmbeddingSpec = modelById.get('discogs_effnet_embedding')
      ?? modelById.get('discogs_effnet');
    const discogsGenreSpec = modelById.get('genre_discogs400');
    const discogsHeadModels = models.filter((model) => model.kind === 'discogsEffnetHead');
    if (discogsEmbeddingSpec) {
      let discogsEmbeddingModel: any = null;
      let discogsGenreModel: any = null;
      let discogsGenreInput: any = null;
      let discogsGenreTensor: any = null;
      try {
        discogsEmbeddingModel = await loadTensorflowModel(tf, discogsEmbeddingSpec);
        let embeddingCount = 0;
        const embeddingSums = new Float64Array(1280);
        let embeddingBatchIndex = 0;
        const melStream = streamDiscogsEffnetMelBatches(
          analysisEssentia,
          signal,
          ({ processedPatches, totalPatches }) => emitProgress({
            stage: 'extractingDiscogs',
            modelFamily: 'discogsEffnet',
            modelId: 'discogs_effnet_embedding',
            message: `正在提取 Discogs-EffNet 特征 ${processedPatches}/${totalPatches}`,
            processed: processedPatches,
            total: totalPatches,
          }),
          { tensorRuntime: tf },
        );
        // Do not load the five classification heads until the large embedding
        // graph has finished. WebKit otherwise keeps the embedding graph,
        // every head graph, and the current audio/WASM heap resident at once.
        // The embedding rows are small compared with those model graphs, so
        // retaining the numeric rows between the two phases is bounded.
        const discogsEmbeddingRows: number[][] = [];
        for await (const batch of melStream) {
          embeddingBatchIndex += 1;
          const actualCount = batch.validPatches;
          const discogsInput = tf.tensor3d(
            batch.values,
            [batch.batchSize, batch.framesPerPatch, batch.melBands],
            'float32',
          );
          let discogsEmbeddingTensor: any = null;
          try {
            emitProgress({
              stage: 'runningDiscogsEmbedding',
              modelFamily: 'discogsEffnet',
              modelId: discogsEmbeddingSpec.id,
              message: `正在运行 Discogs-EffNet 嵌入第 ${embeddingBatchIndex} 批`,
              processed: embeddingBatchIndex,
              total: undefined,
              patchCount: actualCount,
            });
            const output = executeEssentiaModel(
              tf,
              discogsEmbeddingModel,
              discogsInput,
              discogsEmbeddingSpec.outputName || 'discogs_embedding',
            );
            discogsEmbeddingTensor = Array.isArray(output) ? output[0] : output;
            const rows = await discogsEmbeddingTensor.array();
            const values = new Float32Array(actualCount * 1280);
            let validRows = 0;
            for (let index = 0; index < actualCount; index += 1) {
              const row = rows[index];
              if (!Array.isArray(row) || row.length < 1280) continue;
              const offset = validRows * 1280;
              let valid = true;
              for (let dimension = 0; dimension < 1280; dimension += 1) {
                const value = Number(row[dimension]);
                if (!Number.isFinite(value)) {
                  valid = false;
                  break;
                }
                values[offset + dimension] = value;
              }
              if (valid) {
                for (let dimension = 0; dimension < 1280; dimension += 1) {
                  embeddingSums[dimension] += values[offset + dimension];
                }
                discogsEmbeddingRows.push(
                  Array.from(values.subarray(offset, offset + 1280)),
                );
                validRows += 1;
              }
            }
            embeddingCount += validRows;
          } finally {
            discogsEmbeddingTensor?.dispose?.();
            discogsInput.dispose?.();
          }
        }
        discogsEmbeddingModel.dispose?.();
        discogsEmbeddingModel = null;
        const discogsRun = await runDiscogsEffnetHeads(tf, discogsEmbeddingRows, discogsHeadModels, {
          onProgress: (modelId) => emitProgress({
            stage: 'runningDiscogsHeads',
            modelFamily: 'discogsEffnet',
            modelId,
            message: `正在运行 Discogs-EffNet ${modelId}`,
            processed: embeddingCount,
            total: embeddingCount,
          }),
        });
        if (embeddingCount === 0) {
          throw new Error('Discogs EffNet 未返回有效的 1280 维嵌入');
        }
        const discogsEmbedding = Array.from(embeddingSums, (value) => value / embeddingCount);
        // The legacy 400-class Genre projection is optional and must not turn
        // a successful five-head Discogs run into an all-head failure.
        if (discogsGenreSpec) {
          try {
            discogsGenreModel = await loadTensorflowModel(tf, discogsGenreSpec);
            discogsGenreInput = tf.tensor2d([discogsEmbedding], [1, 1280], 'float32');
            const genreOutput = executeEssentiaModel(
              tf,
              discogsGenreModel,
              discogsGenreInput,
              discogsGenreSpec.outputName || 'discogs_genre',
            );
            discogsGenreTensor = Array.isArray(genreOutput) ? genreOutput[0] : genreOutput;
            const genreScores = averagePredictions(await discogsGenreTensor.array(), discogsGenreSpec.classes.length);
            genre.push(...discogsGenreLabels(discogsGenreSpec.classes.map((label, index) => ({
              label,
              confidence: genreScores[index] ?? Number.NaN,
            }))));
          } catch (error) {
            if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
            filtered.push({
              label: 'genre_discogs400',
              confidence: null,
              reason: error instanceof Error ? error.message : String(error),
            });
          }
        }
        // Keep the shared embedding and all five head statuses together so a
        // missing optional head never erases successful siblings.
        discogsEffnet = discogsRun;
      } catch (error) {
        if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
        discogsEffnet = {
          embeddingModel: 'discogs-effnet-bs64-1',
          embeddingDimensions: 1280,
          inputShape: [64, 128, 96],
          heads: Object.fromEntries([
            ['moodTheme', 'discogs_mood_theme'],
            ['approachability', 'discogs_approachability'],
            ['instrumentation', 'discogs_instrumentation'],
            ['timbre', 'discogs_timbre'],
            ['danceability', 'discogs_danceability'],
          ].map(([id, model]) => [id, {
            model: id,
            status: 'failed',
            version: discogsEmbeddingSpec.version,
            labels: [],
            scores: {},
            frameCount: 0,
            reason: error instanceof Error ? error.message : String(error),
          }])) as DiscogsEffnetAnalysis['heads'],
        };
        filtered.push({
          label: 'genre_discogs400',
          confidence: null,
          reason: error instanceof Error ? error.message : String(error),
        });
      } finally {
        discogsGenreTensor?.dispose?.();
        discogsGenreInput?.dispose?.();
        discogsGenreModel?.dispose?.();
        discogsEmbeddingModel?.dispose?.();
      }
    } else {
      discogsEffnet = {
        embeddingModel: 'discogs-effnet-bs64-1',
        embeddingDimensions: 1280,
        inputShape: [64, 128, 96],
        heads: Object.fromEntries([
          ['moodTheme', 'discogs_mood_theme'],
          ['approachability', 'discogs_approachability'],
          ['instrumentation', 'discogs_instrumentation'],
          ['timbre', 'discogs_timbre'],
          ['danceability', 'discogs_danceability'],
        ].map(([id, model]) => [id, {
          model: id,
          status: 'model_missing',
          version: '',
          labels: [],
          scores: {},
          frameCount: 0,
          reason: `未安装 ${model}`,
        }])) as DiscogsEffnetAnalysis['heads'],
      };
      filtered.push({
        label: 'genre_discogs400',
        confidence: null,
        reason: 'Discogs EffNet 或 Genre head 未安装',
      });
    }
    for (const model of models.filter((candidate) =>
      candidate.kind !== 'embedding'
      && candidate.kind !== 'genreEmbedding'
      && candidate.kind !== 'genre'
      && candidate.kind !== 'discogsEffnetEmbedding'
      && candidate.kind !== 'discogsEffnetHead'
      && candidate.kind !== 'emotionContinuous'
      && candidate.kind !== 'emotionCluster')) {
      let classifier: any = null;
      let input: any = null;
      let tensor: any = null;
      try {
        classifier = await loadTensorflowModel(tf, model);
        const outputInput = tf.tensor(embeddingRows, [embeddingRows.length, 200], 'float32');
        input = outputInput;
        const output = executeEssentiaModel(tf, classifier, input, model.outputName);
        tensor = Array.isArray(output) ? output[0] : output;
        const predictions = await tensor.array();
        const scores = averagePredictions(predictions, model.classes.length);
        const labels = model.classes.map((label, index) => ({
          label,
          confidence: scores[index] ?? Number.NaN,
        }));
        const result = filterHighLevelLabels(labels);
        if (model.kind === 'mood') {
          mood.push(...result.accepted);
        } else {
          instrument.push(...result.accepted);
        }
        filtered.push(...result.filtered);
      } catch (error) {
        // A single optional head must not discard basic analysis or the
        // labels produced by other heads.
        if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
        filtered.push({
          label: model.id,
          confidence: null,
          reason: error instanceof Error ? error.message : String(error),
        });
      } finally {
        tensor?.dispose?.();
        input?.dispose?.();
        classifier?.dispose?.();
      }
    }

    emitProgress({
      stage: 'runningEmotionHeads',
      message: '正在运行独立情绪模型',
    });
    const emotionModels = new Map(
      models
        .filter((model) => model.kind === 'emotionContinuous' || model.kind === 'emotionCluster')
        .map((model) => [model.id, model]),
    );
    const emotionRun = await runEmotionHeads(tf, embeddingRows, emotionModels, {
      onProgress: (modelId) => emitProgress({
        stage: 'runningEmotionHeads',
        message: `正在运行 ${modelId} 情绪模型`,
        modelId,
      }),
    });
    filtered.push(...emotionRun.failures.map((failure) => ({
      label: failure.model,
      confidence: null,
      reason: failure.reason,
    })));
    const result: HighLevelAnalysis = {
      status: 'failed',
      modelVersion: embedding.version,
      genre,
      style,
      mood,
      instrument,
      emotionCandidates: emotionRun.emotionCandidates,
      moodCluster: emotionRun.moodCluster,
      moodClusterStatus: emotionRun.moodClusterStatus,
      moodClusterReason: emotionRun.moodClusterReason,
      filtered,
      discogsEffnet,
    };
    if (highLevelAnalysisHasRequiredOutputs(result)) {
      result.status = 'completed';
      result.reason = null;
    } else {
      const incomplete = assessTrackAnalysisCompleteness({
        path: '',
        title: '',
        artist: '',
        album: '',
        durationSeconds: 1,
        bpm: 1,
        key: null,
        scale: null,
        keyStrength: null,
        integratedLoudnessLufs: null,
        loudnessRangeLu: null,
        energy: 1,
        danceability: 1,
        beatPositions: [],
        analyzedAt: '',
        analyzer: '',
        analysisVersion: '',
        dropAnalysis: { status: 'skipped' },
        highLevel: result,
      });
      result.reason = incomplete.reasons.join('；') || '必需高级分析阶段未完成';
    }
    return result;
  } catch (error) {
    if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
    return {
      status: 'failed',
      modelVersion: embedding.version,
      reason: error instanceof Error ? error.message : String(error),
    };
  } finally {
    classifierModels.forEach((model) => model?.dispose?.());
  }
}

function normalizedFilenameStem(path: string): string {
  const filename = path.split(/[\\/]/).pop() || path;
  return filename.replace(/\.[^.]+$/, '').trim().replace(/\s+/g, ' ');
}

function isNeteaseSource(path: string): boolean {
  const lowerPath = path.toLocaleLowerCase();
  return /\.ncm$/i.test(path)
    || lowerPath.split(/[\\/]/).some((segment) => segment.includes('netease') || segment.includes('网易云'));
}

export function filenameIdentity(
  path: string,
  neteaseFilenameFormat: NeteaseFilenameFormat = 'title_artist',
): TrackMetadata {
  const stem = normalizedFilenameStem(path);
  const separator = stem.indexOf(' - ');
  const neteaseSource = isNeteaseSource(path);
  if (neteaseSource && /\.ncm$/i.test(path) && neteaseFilenameFormat === 'title_only') {
    return { title: stem, artist: '', album: '' };
  }
  if (separator > 0) {
    const left = stem.slice(0, separator).trim();
    const right = stem.slice(separator + 3).trim();
    const preferTitleArtist = neteaseSource
      ? neteaseFilenameFormat !== 'artist_title'
      : false;
    return {
      title: preferTitleArtist ? left : right,
      artist: preferTitleArtist ? right : left,
      album: '',
    };
  }
  return { title: stem.trim(), artist: '', album: '' };
}

function cleanMetadataValue(value: string | undefined): string {
  return value?.trim() || '';
}

export function resolveTrackMetadata(
  path: string,
  metadata: TrackMetadata | undefined,
  neteaseFilenameFormat: NeteaseFilenameFormat = 'title_artist',
): TrackMetadata {
  const fallback = filenameIdentity(path, neteaseFilenameFormat);
  const title = cleanMetadataValue(metadata?.title);
  const artist = cleanMetadataValue(metadata?.artist);
  const album = cleanMetadataValue(metadata?.album);
  const genre = cleanMetadataValue(metadata?.genre ?? undefined);
  const withGenre = (result: Omit<TrackMetadata, 'genre'>): TrackMetadata =>
    genre ? { ...result, genre } : result;

  if (/\.ncm$/i.test(path) && neteaseFilenameFormat !== 'title_only') {
    const hasSplitName = fallback.artist.length > 0;
    if (hasSplitName) {
      return withGenre({ title: fallback.title, artist: fallback.artist, album });
    }
  }

  if (title && artist && fallback.title && fallback.artist
    && title === fallback.artist && artist === fallback.title) {
    return withGenre({ title: fallback.title, artist: fallback.artist, album });
  }

  return withGenre({
    title: title || fallback.title,
    artist: artist || fallback.artist,
    album,
  });
}

// WebKit's native MP3 decoder can leave decodeAudioData pending forever for a
// malformed/long file. Keep that failure local to the current candidate so the
// per-song Worker lifecycle can terminate it and continue with the next file.
export const AUDIO_DECODE_TIMEOUT_MS = 300_000;

async function decodeAudio(
  bytes: Uint8Array,
  timeoutMs = AUDIO_DECODE_TIMEOUT_MS,
): Promise<AudioBuffer> {
  const AudioContextConstructor = window.AudioContext ||
    (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioContextConstructor) {
    throw new Error('当前系统不支持 Web Audio 音频解码');
  }

  const context = new AudioContextConstructor();
  try {
    // `read_audio_file` already hands us a single contiguous Uint8Array. Use
    // that backing store directly; copying the compressed source adds another
    // peak allocation before WebKit has even produced its AudioBuffer.
    const audioBuffer = bytes.byteOffset === 0
      && bytes.byteLength === bytes.buffer.byteLength
      && bytes.buffer instanceof ArrayBuffer
      ? bytes.buffer
      : bytes.slice().buffer;
    let timeoutHandle: ReturnType<typeof setTimeout> | null = null;
    const decodePromise = context.decodeAudioData(audioBuffer);
    const timeoutPromise = new Promise<AudioBuffer>((_, reject) => {
      timeoutHandle = setTimeout(() => {
        reject(new Error(`音频解码超时（${timeoutMs}ms）`));
      }, Math.max(1, timeoutMs));
    });
    try {
      return await Promise.race([decodePromise, timeoutPromise]);
    } finally {
      if (timeoutHandle !== null) {
        clearTimeout(timeoutHandle);
      }
    }
  } finally {
    await context.close().catch(() => undefined);
  }
}

async function resampleTo44100(buffer: AudioBuffer): Promise<AudioBuffer> {
  if (buffer.sampleRate === 44100) {
    return buffer;
  }

  const channelCount = Math.min(Math.max(buffer.numberOfChannels, 1), 2);
  const frameCount = Math.max(1, Math.ceil(buffer.duration * 44100));
  const offline = new OfflineAudioContext(channelCount, frameCount, 44100);
  const source = offline.createBufferSource();
  source.buffer = buffer;
  source.connect(offline.destination);
  source.start();
  return offline.startRendering();
}

async function downsampleAudioBufferToMono(
  audio: AudioBuffer,
  targetSampleRate: number,
): Promise<Float32Array> {
  if (audio.numberOfChannels === 1 && audio.sampleRate === targetSampleRate) {
    return audio.getChannelData(0).slice();
  }
  const frameCount = Math.max(1, Math.ceil(audio.duration * targetSampleRate));
  const offline = new OfflineAudioContext(1, frameCount, targetSampleRate);
  const source = offline.createBufferSource();
  source.buffer = audio;
  source.connect(offline.destination);
  source.start();
  const rendered = await offline.startRendering();
  return rendered.getChannelData(0).slice();
}

async function prepareDecodedAudio(
  audio: AudioBuffer,
  includeMusicnnSignal: boolean,
  forceChunked = false,
): Promise<DecodedAudioData> {
  const plan = planAnalysisAudio({
    durationSeconds: audio.duration,
    sampleRate: audio.sampleRate,
    channelCount: audio.numberOfChannels,
  });
  if (forceChunked || plan.mode === 'chunked') {
    // A large AudioBuffer is unavoidable for Web Audio's decoder, but do not
    // carry its full-rate stereo channels over the Worker boundary. The
    // compact mono representation is still the complete track; basic
    // analysis consumes it chunk-by-chunk and high-level models use the same
    // 16 kHz signal without a second copy.
    const targetSampleRate = forceChunked
      ? BOUNDED_ANALYSIS_SAMPLE_RATE
      : plan.sampleRate;
    const signal = await downsampleAudioBufferToMono(audio, targetSampleRate);
    return {
      sampleRate: targetSampleRate,
      duration: audio.duration,
      channels: [signal],
      musicnnSignal: includeMusicnnSignal ? signal : null,
      basicAnalysisMode: 'chunked',
    };
  }

  let musicnnSignal: Float32Array | null = null;
  if (includeMusicnnSignal) {
    const [{ InputExtractor }, runtime] = await Promise.all([
      getTensorflowRuntime(),
      getEssentiaRuntime(),
    ]);
    const extractor = new InputExtractor(runtime.wasmModule.EssentiaWASM, 'musicnn', false);
    try {
      // Keep the existing EssentiaTFInputExtractor downsampling path on the
      // UI thread. It uses OfflineAudioContext asynchronously; the expensive
      // frame-wise WASM loop and all TensorFlow execution happen in the Worker.
      musicnnSignal = (await extractor.downsampleAudioBuffer(audio)).slice();
    } finally {
      extractor.delete();
    }
  }
  return {
    sampleRate: audio.sampleRate,
    duration: audio.duration,
    channels: Array.from({ length: Math.min(Math.max(audio.numberOfChannels, 1), 2) }, (_, index) =>
      audio.getChannelData(index).slice()),
    musicnnSignal,
  };
}

type BasicAnalysisResult = {
  bpm: number | null;
  key: string | null;
  scale: string | null;
  keyStrength: number | null;
  integratedLoudnessLufs: number | null;
  loudnessRangeLu: number | null;
  energy: number | null;
  danceability: number | null;
  beatPositions: number[];
  dropLoudnessLufs: number | null;
  dropAnalysis: DropAnalysisDetails;
};

const BASIC_ANALYSIS_CHUNK_SECONDS = 30;
const BASIC_ESSENTIA_SAMPLE_RATE = 44_100;

function basicAnalysisChunkSeconds(durationSeconds: number): number {
  // A 30-second chunk remains the bounded long-track path.  Some medium
  // tracks have unusually expensive native rhythm/key calculations, though;
  // use smaller complete chunks for them so one synchronous call cannot
  // consume the entire five-minute progress watchdog.
  return durationSeconds > 0 && durationSeconds < LONG_TRACK_DURATION_THRESHOLD_SECONDS
    ? 10
    : BASIC_ANALYSIS_CHUNK_SECONDS;
}

function resampleSignalForEssentia(
  signal: Float32Array,
  sourceSampleRate: number,
  targetSampleRate: number,
): Float32Array {
  if (sourceSampleRate === targetSampleRate) return signal;
  const ratio = targetSampleRate / Math.max(1, sourceSampleRate);
  const targetLength = Math.max(1, Math.round(signal.length * ratio));
  const result = new Float32Array(targetLength);
  for (let index = 0; index < targetLength; index += 1) {
    const sourcePosition = index / ratio;
    const lower = Math.min(signal.length - 1, Math.floor(sourcePosition));
    const upper = Math.min(signal.length - 1, lower + 1);
    const fraction = sourcePosition - lower;
    result[index] = signal[lower] * (1 - fraction) + signal[upper] * fraction;
  }
  return result;
}

function median(values: number[]): number | null {
  const finite = values.filter((value) => Number.isFinite(value)).sort((left, right) => left - right);
  if (finite.length === 0) return null;
  const middle = Math.floor(finite.length / 2);
  return finite.length % 2 === 0
    ? (finite[middle - 1] + finite[middle]) / 2
    : finite[middle];
}

function chunkBeatLoudness(
  essentia: EssentiaInstance,
  audio: any,
  beatPositions: number[],
): any {
  let beatVector: any;
  let frequencyBandsVector: any;
  try {
    beatVector = essentia.arrayToVector(Float32Array.from(beatPositions));
    frequencyBandsVector = essentia.arrayToVector(
      Float32Array.from([20, 150, 400, 3200, 7000, 22000]),
    );
    if (essentia.algorithms?.BeatsLoudness) {
      return essentia.algorithms.BeatsLoudness(
        audio,
        0.05,
        0.1,
        beatVector,
        frequencyBandsVector,
        BASIC_ESSENTIA_SAMPLE_RATE,
      );
    }
    return essentia.BeatsLoudness(
      audio,
      0.05,
      0.1,
      beatPositions,
      [20, 150, 400, 3200, 7000, 22000],
      BASIC_ESSENTIA_SAMPLE_RATE,
    );
  } finally {
    releaseVector(beatVector);
    releaseVector(frequencyBandsVector);
  }
}

/**
 * Analyze the complete bounded signal without ever creating a full-duration
 * Essentia vector. Each 30-second window contributes to the basic metrics;
 * beat positions are translated back to the original track timeline.
 */
export async function analyzeChunkedBasicAudio(
  essentia: EssentiaInstance,
  audio: DecodedAudioData,
  onProgress?: (progress: AnalysisWorkerProgress) => void,
  options: { chunkSeconds?: number } = {},
): Promise<BasicAnalysisResult> {
  const signal = audio.channels[0] ?? new Float32Array();
  const sourceSampleRate = Math.max(1, Math.trunc(audio.sampleRate) || BOUNDED_ANALYSIS_SAMPLE_RATE);
  const chunkSeconds = Number.isFinite(options.chunkSeconds) && (options.chunkSeconds ?? 0) > 0
    ? options.chunkSeconds as number
    : BASIC_ANALYSIS_CHUNK_SECONDS;
  const chunkSize = Math.max(1, Math.floor(sourceSampleRate * chunkSeconds));
  const bpms: number[] = [];
  const keyVotes = new Map<string, { key: string; scale: string; strength: number; weight: number }>();
  const beatPositions: number[] = [];
  const beatLoudness: number[] = [];
  let totalEnergy = 0;
  let totalEnergySamples = 0;
  let loudnessPower = 0;
  let loudnessDuration = 0;
  let loudnessRangeTotal = 0;
  let loudnessRangeDuration = 0;
  let danceabilityTotal = 0;
  let danceabilityDuration = 0;
  const duration = Number.isFinite(audio.duration) && audio.duration > 0
    ? audio.duration
    : signal.length / sourceSampleRate;

  for (let start = 0; start < signal.length; start += chunkSize) {
    const end = Math.min(signal.length, start + chunkSize);
    const sourceChunk = signal.subarray(start, end);
    const analysisChunk = resampleSignalForEssentia(
      sourceChunk,
      sourceSampleRate,
      BASIC_ESSENTIA_SAMPLE_RATE,
    );
    const chunkEssentia = essentia.createInstance?.() ?? essentia;
    let analysisVector: any = null;
    let rhythm: any = null;
    let key: any = null;
    let loudness: any = null;
    let danceabilityResult: any = null;
    let beatsLoudness: any = null;
    const safe = <T>(operation: () => T): T | null => {
      try {
        return operation();
      } catch (error) {
        if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
        return null;
      }
    };
    try {
      analysisVector = chunkEssentia.arrayToVector(analysisChunk);
      rhythm = safe(() => chunkEssentia.RhythmExtractor2013(analysisVector, 208, 'multifeature', 40));
      const localBeatPositions = vectorToNumbers(chunkEssentia, rhythm?.ticks);
      const offsetSeconds = start / sourceSampleRate;
      const beatPairs = localBeatPositions
        .map((position) => ({
          local: position,
          global: position + offsetSeconds,
        }))
        .filter(({ global }) => Number.isFinite(global) && global >= 0 && global < duration);
      const validLocalBeatPositions = beatPairs.map(({ local }) => local);
      const globalBeatPositions = beatPairs.map(({ global }) => global);
      beatPositions.push(...globalBeatPositions);
      if (globalBeatPositions.length > 0) {
        beatsLoudness = safe(() => chunkBeatLoudness(
          chunkEssentia,
          analysisVector,
          validLocalBeatPositions,
        ));
        const localBeatValues = vectorToNumbers(chunkEssentia, beatsLoudness?.loudness);
        beatLoudness.push(...globalBeatPositions.map((_, index) => localBeatValues[index] ?? Number.NaN));
      }

      const bpm = finiteNumber(rhythm?.bpm);
      if (bpm !== null) bpms.push(bpm);
      key = safe(() => chunkEssentia.KeyExtractor(
        analysisVector,
        true,
        4096,
        4096,
        12,
        3500,
        60,
        25,
        0.2,
        'bgate',
        BASIC_ESSENTIA_SAMPLE_RATE,
        0.0001,
        440,
        'cosine',
        'hann',
      ));
      const keyName = typeof key?.key === 'string' ? key.key : '';
      const scaleName = typeof key?.scale === 'string' ? key.scale : '';
      if (keyName) {
        const strength = Math.max(0, finiteNumber(key?.strength) ?? 0);
        const identity = `${keyName}\u0000${scaleName}`;
        const previous = keyVotes.get(identity) ?? {
          key: keyName,
          scale: scaleName,
          strength: 0,
          weight: 0,
        };
        previous.strength += strength;
        previous.weight += 1;
        keyVotes.set(identity, previous);
      }

      loudness = safe(() => chunkEssentia.LoudnessEBUR128(
        analysisVector,
        analysisVector,
        0.1,
        BASIC_ESSENTIA_SAMPLE_RATE,
        false,
      ));
      const chunkDuration = Math.max(0, (end - start) / sourceSampleRate);
      const chunkLufs = finiteNumber(loudness?.integratedLoudness);
      if (chunkLufs !== null) {
        loudnessPower += 10 ** (chunkLufs / 10) * chunkDuration;
        loudnessDuration += chunkDuration;
      }
      const chunkRange = finiteNumber(loudness?.loudnessRange);
      if (chunkRange !== null) {
        loudnessRangeTotal += chunkRange * chunkDuration;
        loudnessRangeDuration += chunkDuration;
      }
      danceabilityResult = safe(() => chunkEssentia.Danceability(
        analysisVector,
        8800,
        310,
        BASIC_ESSENTIA_SAMPLE_RATE,
        1.1,
      ));
      const danceability = finiteNumber(danceabilityResult?.danceability);
      if (danceability !== null) {
        danceabilityTotal += danceability * chunkDuration;
        danceabilityDuration += chunkDuration;
      }
    } finally {
      releaseVector(rhythm?.ticks);
      releaseVector(rhythm?.estimates);
      releaseVector(rhythm?.bpmIntervals);
      releaseVector(beatsLoudness?.loudness);
      releaseVector(beatsLoudness?.loudnessBandRatio);
      releaseVector(loudness?.momentaryLoudness);
      releaseVector(loudness?.shortTermLoudness);
      releaseVector(danceabilityResult?.dfa);
      releaseVector(analysisVector);
      if (chunkEssentia !== essentia) {
        chunkEssentia.flushPendingDeletes?.();
        chunkEssentia.delete();
        chunkEssentia.flushPendingDeletes?.();
      }
    }

    for (const value of sourceChunk) {
      if (Number.isFinite(value)) {
        totalEnergy += value * value;
        totalEnergySamples += 1;
      }
    }
    onProgress?.({
      stage: 'analyzingBasic',
      message: `正在计算整曲基础分析 ${end}/${signal.length}`,
      processed: end,
      total: signal.length,
    });
    if (end < signal.length) {
      // Rhythm/key/loudness are synchronous native calls. Give the host a
      // longer scheduling window after each chunk so aggregate CPU remains
      // bounded even when a native call briefly uses one full core.
      await yieldToAnalysisWorker(2500);
    }
  }

  let selectedKey: { key: string; scale: string; strength: number; weight: number } | undefined;
  for (const value of keyVotes.values()) {
    if (!selectedKey || value.weight > selectedKey.weight
      || (value.weight === selectedKey.weight && value.strength > selectedKey.strength)) {
      selectedKey = value;
    }
  }
  const integratedLoudnessLufs = loudnessDuration > 0
    ? 10 * Math.log10(Math.max(Number.EPSILON, loudnessPower / loudnessDuration))
    : null;
  const selected = selectDropBeatWindow(beatPositions, beatLoudness, duration);
  let dropLoudnessLufs: number | null = null;
  let dropAnalysis: DropAnalysisDetails = {
    status: 'skipped',
    reason: beatPositions.length >= DROP_BEAT_COUNT
      ? '头尾 15% 排除后不足 32 个有效 Beat，或 Beat loudness 无效'
      : '没有足够的 Beat positions',
  };
  if (selected) {
    const startSeconds = beatPositions[selected.startIndex];
    const bpm = median(bpms);
    const beatDurationSeconds = bpm !== null ? 60 / Math.max(1, bpm) : 0;
    const nextBeatSeconds = beatPositions
      .slice(selected.endIndex + 1)
      .find((position) => Number.isFinite(position));
    const selectedEndSeconds = beatPositions[selected.endIndex];
    const endSeconds = Math.min(
      duration,
      nextBeatSeconds ?? (Number.isFinite(selectedEndSeconds)
        ? selectedEndSeconds + beatDurationSeconds
        : Number.NaN),
    );
    const startFrame = Number.isFinite(startSeconds)
      ? Math.max(0, Math.floor(startSeconds * sourceSampleRate))
      : Number.NaN;
    const endFrame = Number.isFinite(endSeconds)
      ? Math.min(signal.length, Math.ceil(endSeconds * sourceSampleRate))
      : Number.NaN;
    if (!Number.isFinite(startFrame) || !Number.isFinite(endFrame) || endFrame <= startFrame) {
      dropAnalysis = { status: 'skipped', reason: '无法截取有效 Drop 音频片段' };
    } else {
      const dropSignal = resampleSignalForEssentia(
        signal.subarray(startFrame, endFrame),
        sourceSampleRate,
        BASIC_ESSENTIA_SAMPLE_RATE,
      );
      const dropVector = essentia.arrayToVector(dropSignal);
      let dropLoudness: any = null;
      try {
        dropLoudness = (() => {
          try {
            return essentia.LoudnessEBUR128(
              dropVector,
              dropVector,
              0.1,
              BASIC_ESSENTIA_SAMPLE_RATE,
              false,
            );
          } catch (error) {
            if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
            return null;
          }
        })();
        dropLoudnessLufs = finiteNumber(dropLoudness?.integratedLoudness);
        dropAnalysis = dropLoudnessLufs === null
          ? { status: 'failed', reason: 'Drop LUFS 计算失败' }
          : {
            status: 'completed',
            beatStartIndex: selected.startIndex,
            beatEndIndex: selected.endIndex,
            beatCount: DROP_BEAT_COUNT,
            segmentStartSeconds: startSeconds,
            segmentEndSeconds: endSeconds,
            selectedAverageBeatLoudness: selected.averageLoudness,
          };
      } finally {
        releaseVector(dropLoudness?.momentaryLoudness);
        releaseVector(dropLoudness?.shortTermLoudness);
        releaseVector(dropVector);
      }
    }
  }
  return {
    bpm: median(bpms),
    key: selectedKey?.key ?? null,
    scale: selectedKey?.scale ?? null,
    keyStrength: selectedKey ? selectedKey.strength / Math.max(1, selectedKey.weight) : null,
    integratedLoudnessLufs,
    loudnessRangeLu: loudnessRangeDuration > 0
      ? loudnessRangeTotal / loudnessRangeDuration
      : null,
    energy: totalEnergySamples > 0 ? Math.max(0, totalEnergy / totalEnergySamples) : null,
    danceability: danceabilityDuration > 0
      ? danceabilityTotal / danceabilityDuration
      : null,
    beatPositions,
    dropLoudnessLufs,
    dropAnalysis,
  };
}

export async function analyzeDecodedAudio(
  path: string,
  audio: DecodedAudioData,
  metadata?: TrackMetadata,
  options: {
    fingerprint?: AnalysisFingerprint;
    neteaseFilenameFormat?: NeteaseFilenameFormat;
    highLevel?: HighLevelAnalysis;
    highLevelModels?: EssentiaModelFile[];
    tensorflowBackend?: 'cpu' | 'webgl' | 'wasm';
    onProgress?: (progress: AnalysisWorkerProgress) => void;
  } = {},
): Promise<TrackAnalysis> {
  const neteaseFilenameFormat = options.neteaseFilenameFormat ?? 'title_artist';
  const resolvedMetadata = resolveTrackMetadata(path, metadata, neteaseFilenameFormat);
  const fallbackMetadata = filenameIdentity(path, neteaseFilenameFormat);
  const essentia = await getEssentia();
  if (audio.basicAnalysisMode === 'chunked') {
    const basic = await analyzeChunkedBasicAudio(essentia, audio, options.onProgress, {
      chunkSeconds: basicAnalysisChunkSeconds(audio.duration),
    });
    let highLevel = options.highLevel;
    if (!highLevel && options.highLevelModels && options.highLevelModels.length > 0) {
      options.onProgress?.({
        stage: 'analyzingHighLevel',
        message: '正在运行 Essentia 预训练模型',
      });
      try {
        highLevel = await runHighLevelAnalysis(
          audio,
          options.highLevelModels,
          options.onProgress,
          options.tensorflowBackend,
        );
      } catch (error) {
        if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
        highLevel = {
          status: 'failed',
          modelVersion: options.highLevelModels[0]?.version,
          reason: error instanceof Error ? error.message : String(error),
        };
      }
    }
    return {
      path,
      title: resolvedMetadata.title || fallbackMetadata.title,
      artist: resolvedMetadata.artist || fallbackMetadata.artist,
      album: resolvedMetadata.album,
      genre: resolvedMetadata.genre || fallbackMetadata.genre || '',
      durationSeconds: finiteNumber(audio.duration),
      bpm: basic.bpm,
      key: basic.key,
      scale: basic.scale,
      keyStrength: basic.keyStrength,
      integratedLoudnessLufs: basic.integratedLoudnessLufs,
      loudnessRangeLu: basic.loudnessRangeLu,
      energy: basic.energy,
      danceability: basic.danceability,
      beatPositions: basic.beatPositions,
      analyzedAt: new Date().toISOString(),
      analyzer: 'Essentia.js',
      analysisVersion: TRACK_ANALYSIS_VERSION,
      sourceSizeBytes: options.fingerprint?.sizeBytes ?? null,
      sourceModifiedAt: options.fingerprint?.modifiedAt ?? null,
      sourceFilenameFormat: neteaseFilenameFormat,
      dropLoudnessLufs: basic.dropLoudnessLufs,
      dropAnalysis: basic.dropAnalysis,
      highLevel: highLevel ?? {
        status: 'model_missing',
        reason: '未下载 Essentia 预训练模型',
      },
    };
  }
  const sampleRate = audio.sampleRate;
  const left = audio.channels[0] ?? new Float32Array();
  const right = audio.channels.length > 1 ? audio.channels[1] : left;
  const leftVector = essentia.arrayToVector(left);
  const rightVector = audio.channels.length > 1 ? essentia.arrayToVector(right) : leftVector;
  let monoVector: any;

  try {
    const mixed = audio.channels.length > 1
      ? essentia.MonoMixer(leftVector, rightVector)
      : null;
    const mono = mixed ? essentia.vectorToArray(mixed.audio) : left;
    releaseVector(mixed?.audio);
    monoVector = mixed ? essentia.arrayToVector(mono) : leftVector;
    const safe = <T>(operation: () => T): T | null => {
      try {
        return operation();
      } catch (error) {
        if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
        return null;
      }
    };
    const rhythm = safe(() => essentia.RhythmExtractor2013(monoVector, 208, 'multifeature', 40));
    const key = safe(() => essentia.KeyExtractor(monoVector));
    const loudness = safe(() => essentia.LoudnessEBUR128(leftVector, rightVector, 0.1, sampleRate, false));
    const energyResult = safe(() => essentia.Energy(monoVector));
    const danceabilityResult = safe(() => essentia.Danceability(monoVector));
    const beatPositions = vectorToNumbers(essentia, rhythm?.ticks);
    releaseVector(rhythm?.ticks);
    const energy = finiteNumber(energyResult?.energy);
    let dropLoudnessLufs: number | null = null;
    let dropAnalysis: DropAnalysisDetails = {
      status: 'skipped',
      reason: '没有足够的 Beat positions',
    };
    if (beatPositions.length >= DROP_BEAT_COUNT) {
      let beatVector: any;
      let frequencyBandsVector: any;
      try {
        beatVector = essentia.arrayToVector(Float32Array.from(beatPositions));
        frequencyBandsVector = essentia.arrayToVector(
          Float32Array.from([20, 150, 400, 3200, 7000, 22000]),
        );
        const beatsLoudness = safe(() => {
          if (essentia.algorithms?.BeatsLoudness) {
            return essentia.algorithms.BeatsLoudness(
              monoVector,
              0.05,
              0.1,
              beatVector,
              frequencyBandsVector,
              sampleRate,
            );
          }
          return essentia.BeatsLoudness(
            monoVector,
            0.05,
            0.1,
            beatPositions,
            [20, 150, 400, 3200, 7000, 22000],
            sampleRate,
          );
        });
        const beatLoudness = vectorToNumbers(essentia, beatsLoudness?.loudness);
        const selected = selectDropBeatWindow(beatPositions, beatLoudness, audio.duration);
        if (!selected) {
          dropAnalysis = {
            status: 'skipped',
            reason: '头尾 15% 排除后不足 32 个有效 Beat，或 Beat loudness 无效',
          };
        } else {
          const startSeconds = beatPositions[selected.startIndex];
          const beatDurationSeconds = finiteNumber(rhythm?.bpm)
            ? 60 / Math.max(1, finiteNumber(rhythm?.bpm) || 1)
            : 0;
          const nextBeatSeconds = beatPositions
            .slice(selected.endIndex + 1)
            .find((position) => Number.isFinite(position));
          const selectedEndSeconds = beatPositions[selected.endIndex];
          const endSeconds = Math.min(
            audio.duration,
            nextBeatSeconds
              ?? (Number.isFinite(selectedEndSeconds)
                ? selectedEndSeconds + beatDurationSeconds
                : Number.NaN),
          );
          const startFrame = Number.isFinite(startSeconds)
            ? Math.max(0, Math.floor(startSeconds * sampleRate))
            : Number.NaN;
          const endFrame = Number.isFinite(endSeconds)
            ? Math.min(left.length, Math.ceil(endSeconds * sampleRate))
            : Number.NaN;
          if (!Number.isFinite(startFrame) || !Number.isFinite(endFrame) || endFrame <= startFrame) {
            dropAnalysis = { status: 'skipped', reason: '无法截取有效 Drop 音频片段' };
          } else {
            let dropLeftVector: any;
            let dropRightVector: any;
            try {
              dropLeftVector = essentia.arrayToVector(left.slice(startFrame, endFrame));
              dropRightVector = essentia.arrayToVector(right.slice(startFrame, endFrame));
              const dropLoudness = safe(() => essentia.LoudnessEBUR128(
                dropLeftVector,
                dropRightVector,
                0.1,
                sampleRate,
                false,
              ));
              dropLoudnessLufs = finiteNumber(dropLoudness?.integratedLoudness);
              dropAnalysis = dropLoudnessLufs === null
                ? { status: 'failed', reason: 'Drop LUFS 计算失败' }
                : {
                  status: 'completed',
                  beatStartIndex: selected.startIndex,
                  beatEndIndex: selected.endIndex,
                  beatCount: DROP_BEAT_COUNT,
                  segmentStartSeconds: startSeconds,
                  segmentEndSeconds: endSeconds,
                  selectedAverageBeatLoudness: selected.averageLoudness,
                };
            } finally {
              releaseVector(dropLeftVector);
              releaseVector(dropRightVector);
            }
          }
        }
        releaseVector(beatsLoudness?.loudness);
        releaseVector(beatsLoudness?.loudnessBandRatio);
      } catch (error) {
        if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
        dropAnalysis = {
          status: 'failed',
          reason: error instanceof Error ? error.message : 'BeatsLoudness 计算失败',
        };
      } finally {
        releaseVector(beatVector);
        releaseVector(frequencyBandsVector);
      }
    }
    let highLevel = options.highLevel;
    if (!highLevel && options.highLevelModels && options.highLevelModels.length > 0) {
      options.onProgress?.({
        stage: 'analyzingHighLevel',
        message: '正在运行 Essentia 预训练模型',
      });
      try {
        highLevel = await runHighLevelAnalysis(
          audio,
          options.highLevelModels,
          options.onProgress,
          options.tensorflowBackend,
        );
      } catch (error) {
        // Basic analysis is useful on its own. An unexpected model/runtime
        // failure must not discard the values already computed above or turn
        // the output metadata write-back into an all-or-nothing operation.
        if (isFatalAnalysisRuntimeMessage(analysisErrorMessage(error))) throw error;
        highLevel = {
          status: 'failed',
          modelVersion: options.highLevelModels[0]?.version,
          reason: error instanceof Error ? error.message : String(error),
        };
      }
    }
    return {
      path,
      title: resolvedMetadata.title || fallbackMetadata.title,
      artist: resolvedMetadata.artist || fallbackMetadata.artist,
      album: resolvedMetadata.album,
      genre: resolvedMetadata.genre || fallbackMetadata.genre || '',
      durationSeconds: finiteNumber(audio.duration),
      bpm: finiteNumber(rhythm?.bpm),
      key: typeof key?.key === 'string' ? key.key : null,
      scale: typeof key?.scale === 'string' ? key.scale : null,
      keyStrength: finiteNumber(key?.strength),
      integratedLoudnessLufs: finiteNumber(loudness?.integratedLoudness),
      loudnessRangeLu: finiteNumber(loudness?.loudnessRange),
      energy: energy === null ? null : Math.max(0, energy / Math.max(1, mono.length)),
      danceability: finiteNumber(danceabilityResult?.danceability),
      beatPositions,
      analyzedAt: new Date().toISOString(),
      analyzer: 'Essentia.js',
      analysisVersion: TRACK_ANALYSIS_VERSION,
      sourceSizeBytes: options.fingerprint?.sizeBytes ?? null,
      sourceModifiedAt: options.fingerprint?.modifiedAt ?? null,
      sourceFilenameFormat: neteaseFilenameFormat,
      dropLoudnessLufs,
      dropAnalysis,
      highLevel: highLevel ?? {
        status: 'model_missing',
        reason: '未下载 Essentia 预训练模型',
      },
    };
  } finally {
    const released = new Set<any>();
    for (const vector of [monoVector, leftVector, rightVector]) {
      if (vector && !released.has(vector)) {
        released.add(vector);
        releaseVector(vector);
      }
    }
  }
}

export async function analyzeAudioFile(
  path: string,
  bytes: Uint8Array,
  metadata?: TrackMetadata,
  options: {
    fingerprint?: AnalysisFingerprint;
    neteaseFilenameFormat?: NeteaseFilenameFormat;
    highLevel?: HighLevelAnalysis;
    highLevelModels?: EssentiaModelFile[];
    tensorflowBackend?: 'cpu' | 'webgl' | 'wasm';
    workerClient?: AnalysisWorkerClientLike;
    workerJobId?: string;
    timeoutMs?: number;
    /** Headless acceptance can opt into chunked basic analysis even for short
     * tracks so native full-track algorithms never monopolize WebContent. */
    forceChunked?: boolean;
    onProgress?: (progress: AnalysisWorkerProgress) => void;
  } = {},
): Promise<TrackAnalysis> {
  if (!options.workerClient) {
    throw new Error('增强分析 Worker 未初始化');
  }
  // Release the Web Audio AudioBuffer before waiting for the Worker.  A long
  // stereo track otherwise stays alive on the WebKit content process while
  // the Worker owns the transferred PCM, doubling peak memory and allowing
  // WebKit to restart the page during the first long-song analysis.
  let decodedAudio: AudioBuffer | null = await decodeAudio(
    bytes,
    Math.min(AUDIO_DECODE_TIMEOUT_MS, options.timeoutMs ?? AUDIO_DECODE_TIMEOUT_MS),
  );
  const plan = planAnalysisAudio({
    durationSeconds: decodedAudio.duration,
    sampleRate: decodedAudio.sampleRate,
    channelCount: decodedAudio.numberOfChannels,
  });
  if (plan.mode === 'native') {
    decodedAudio = await resampleTo44100(decodedAudio);
  }
  const prepared = await prepareDecodedAudio(
    decodedAudio,
    Boolean(options.highLevelModels?.length),
    options.forceChunked ?? false,
  );
  decodedAudio = null;
  return options.workerClient.analyze({
    jobId: options.workerJobId ?? 'analysis',
    path,
    metadata,
    fingerprint: options.fingerprint,
    neteaseFilenameFormat: options.neteaseFilenameFormat ?? 'title_artist',
    highLevel: options.highLevel,
    audio: prepared,
    onProgress: options.onProgress,
    timeoutMs: options.timeoutMs ?? analysisTimeoutMs(prepared.duration),
  });
}
