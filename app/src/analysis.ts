import type { AnalysisWorkerProgress } from './analysis-worker-protocol';
import { analysisTimeoutMs } from './analysis-timeout';
import { runEmotionHeads } from './emotion-models';
import { runDiscogsEffnetHeadsStream } from './discogs-effnet';

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
};

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
  const values = new Float32Array(batchCount * safePatchSize * safeMelBands);
  const copyFrames = Math.min(safeFrameCount, Math.floor(melBuffer.length / safeMelBands));
  for (let frame = 0; frame < copyFrames; frame += 1) {
    const offset = frame * safeMelBands;
    values.set(melBuffer.subarray(offset, offset + safeMelBands), offset);
  }
  return { values, batchCount };
}

export const MUSICCNN_INFERENCE_BATCH_SIZE = 64;

export function musicCnnInferenceBatches(
  patchCount: number,
): Array<{ offset: number; validPatches: number }> {
  const safePatchCount = Math.max(0, Math.trunc(patchCount));
  const batches: Array<{ offset: number; validPatches: number }> = [];
  for (let offset = 0; offset < safePatchCount; offset += MUSICCNN_INFERENCE_BATCH_SIZE) {
    batches.push({
      offset,
      validPatches: Math.min(MUSICCNN_INFERENCE_BATCH_SIZE, safePatchCount - offset),
    });
  }
  return batches;
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
  KeyExtractor: (audio: any) => any;
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
  Danceability: (audio: any) => any;
  delete: () => void;
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
const MUSICNN_FRAME_CHUNK_SIZE = 256;
const DISCOGS_EFFNET_FRAME_SIZE = 512;
const DISCOGS_EFFNET_HOP_SIZE = 256;
const DISCOGS_EFFNET_PATCH_SIZE = 128;
const DISCOGS_EFFNET_MEL_BANDS = 96;
const DISCOGS_EFFNET_BATCH_SIZE = 64;

function yieldToAnalysisWorker(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
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
): Promise<MusicnnMelFeatures> {
  const melRows: number[][] = [];
  // FrameGenerator materializes every frame in a native vector. The bundled
  // Embind build can also retain temporary FFT/Mel allocations until its
  // Essentia instance is destroyed. In the desktop WebView a full song can
  // therefore grow the WASM heap into gigabytes. When a runtime factory is
  // available, process bounded chunks and destroy each short-lived instance;
  // lightweight test doubles continue to use the single-instance fallback.
  const expectedTotal = signal.length >= MUSICNN_FRAME_SIZE
    ? Math.floor((signal.length - MUSICNN_FRAME_SIZE) / MUSICNN_HOP_SIZE) + 1
    : 0;
  let total = expectedTotal;
  let melBuffer = new Float32Array(total * MUSICNN_MEL_BANDS);

  const appendFrame = (frameEssentia: EssentiaInstance, frame: any, index: number) => {
    try {
      const output = frameEssentia.TensorflowInputMusiCNN(frame);
      const bands = output?.bands;
      try {
        const values = Array.from(frameEssentia.vectorToArray(bands));
        melRows.push(values);
        const offset = index * MUSICNN_MEL_BANDS;
        for (let band = 0; band < Math.min(values.length, MUSICNN_MEL_BANDS); band += 1) {
          const value = Number(values[band]);
          melBuffer[offset + band] = Number.isFinite(value) ? value : 0;
        }
      } finally {
        releaseVector(bands);
        releaseVector(output);
      }
    } finally {
      releaseVector(frame);
    }
  };

  if (essentia.createInstance && expectedTotal > 0) {
    for (let chunkStart = 0; chunkStart < expectedTotal; chunkStart += MUSICNN_FRAME_CHUNK_SIZE) {
      const chunkTotal = Math.min(MUSICNN_FRAME_CHUNK_SIZE, expectedTotal - chunkStart);
      const signalStart = chunkStart * MUSICNN_HOP_SIZE;
      const signalEnd = Math.min(
        signal.length,
        signalStart + (chunkTotal - 1) * MUSICNN_HOP_SIZE + MUSICNN_FRAME_SIZE,
      );
      const chunkEssentia = essentia.createInstance();
      let frames: any = null;
      try {
        frames = chunkEssentia.FrameGenerator(
          signal.subarray(signalStart, signalEnd),
          MUSICNN_FRAME_SIZE,
          MUSICNN_HOP_SIZE,
        );
        const actualChunkTotal = Math.min(chunkTotal, Math.max(0, Number(frames?.size?.() ?? 0)));
        for (let localIndex = 0; localIndex < actualChunkTotal; localIndex += 1) {
          appendFrame(chunkEssentia, frames.get(localIndex), chunkStart + localIndex);
          const processed = chunkStart + localIndex + 1;
          if (processed === total || processed % MUSICNN_PROGRESS_BATCH === 0) {
            onProgress?.({ processed, total });
          }
        }
      } finally {
        releaseVector(frames);
        chunkEssentia.delete();
      }
      if (chunkStart + chunkTotal < total) {
        await yieldToAnalysisWorker();
      }
    }
  } else {
    const frames = essentia.FrameGenerator(signal, MUSICNN_FRAME_SIZE, MUSICNN_HOP_SIZE);
    try {
      total = Math.max(0, Number(frames?.size?.() ?? 0));
      melBuffer = new Float32Array(total * MUSICNN_MEL_BANDS);
      for (let index = 0; index < total; index += 1) {
        appendFrame(essentia, frames.get(index), index);
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
): AsyncGenerator<DiscogsEffnetMelBatch> {
  const frames = essentia.FrameGenerator(
    signal,
    DISCOGS_EFFNET_FRAME_SIZE,
    DISCOGS_EFFNET_HOP_SIZE,
  );
  const patchRows: number[][] = [];
  let batchValues = new Float32Array(
    DISCOGS_EFFNET_BATCH_SIZE * DISCOGS_EFFNET_PATCH_SIZE * DISCOGS_EFFNET_MEL_BANDS,
  );
  let validPatches = 0;
  let processedPatches = 0;
  const extract = essentia.TensorflowInputDiscogsEffNet;
  if (!extract) {
    throw new Error('Essentia.js 未提供 Discogs-EffNet Mel 前端');
  }
  try {
    const totalFrames = Math.max(0, Number(frames?.size?.() ?? 0));
    const totalPatches = Math.max(1, Math.ceil(totalFrames / DISCOGS_EFFNET_PATCH_SIZE));
    for (let index = 0; index < totalFrames; index += 1) {
      const frame = frames.get(index);
      try {
        const output = extract(frame);
        const bands = output?.bands;
        try {
          const values = Array.from(essentia.vectorToArray(bands))
            .slice(0, DISCOGS_EFFNET_MEL_BANDS)
            .map((value) => Number(value));
          patchRows.push(Array.from({ length: DISCOGS_EFFNET_MEL_BANDS }, (_, band) =>
            Number.isFinite(values[band]) ? values[band] : 0));
        } finally {
          releaseVector(bands);
          releaseVector(output);
        }
      } finally {
        releaseVector(frame);
      }
      const patchComplete = patchRows.length === DISCOGS_EFFNET_PATCH_SIZE
        || index + 1 === totalFrames;
      if (patchComplete) {
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
        if (validPatches === DISCOGS_EFFNET_BATCH_SIZE || processedPatches === totalPatches) {
          onProgress?.({ processedPatches, totalPatches });
          yield {
            values: batchValues,
            batchSize: DISCOGS_EFFNET_BATCH_SIZE,
            framesPerPatch: DISCOGS_EFFNET_PATCH_SIZE,
            melBands: DISCOGS_EFFNET_MEL_BANDS,
            validPatches,
          };
          if (processedPatches < totalPatches) {
            await yieldToAnalysisWorker();
          }
          batchValues = new Float32Array(
            DISCOGS_EFFNET_BATCH_SIZE * DISCOGS_EFFNET_PATCH_SIZE * DISCOGS_EFFNET_MEL_BANDS,
          );
          validPatches = 0;
        }
      }
      if ((index + 1) % 32 === 0 && !patchComplete) {
        await yieldToAnalysisWorker();
      }
    }
    if (totalFrames === 0) {
      onProgress?.({ processedPatches: 1, totalPatches: 1 });
      yield {
        values: batchValues,
        batchSize: DISCOGS_EFFNET_BATCH_SIZE,
        framesPerPatch: DISCOGS_EFFNET_PATCH_SIZE,
        melBands: DISCOGS_EFFNET_MEL_BANDS,
        validPatches: 1,
      };
    }
  } finally {
    releaseVector(frames);
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
      const createInstance = (): EssentiaInstance => {
        const instance = new Constructor(wasmModule.EssentiaWASM, false);
        if (!instance.TensorflowInputDiscogsEffNet) {
          instance.TensorflowInputDiscogsEffNet = (frame: any) =>
            instance.TensorflowInputMusiCNN(frame);
        }
        return instance;
      };
      const essentia = createInstance();
      essentia.createInstance = createInstance;
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
  } catch {
    return [];
  }
}

function releaseVector(value: any): void {
  if (value && typeof value.delete === 'function') {
    value.delete();
  }
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
 * Native WebKit needs an explicit backend selection.  With one Worker per
 * song and TensorFlow scopes around every graph call, WebGL is now the fast
 * and bounded option; CPU fallback remains available when WebGL is not
 * registered by the embedded runtime.  The helper is deliberately injectable
 * for deterministic tests and future WebView user-agent changes.
 */
export function shouldUseCpuTensorflowBackend(userAgent: string): boolean {
  return /AppleWebKit/i.test(userAgent)
    && !/(Chrome|Chromium|CriOS|Edg|OPR)/i.test(userAgent);
}

export async function configureTensorflowBackend(
  tf: any,
  userAgent = typeof navigator === 'undefined' ? '' : navigator.userAgent,
): Promise<string | undefined> {
  if (shouldUseCpuTensorflowBackend(userAgent) && typeof tf.setBackend === 'function') {
    const selected = await tf.setBackend('webgl');
    if (selected === false) {
      const fallback = await tf.setBackend('cpu');
      if (fallback === false) {
        throw new Error('无法为 WebKit 启用 TensorFlow.js WebGL/CPU 后端');
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
): Promise<HighLevelAnalysis> {
  const modelById = new Map(models.map((model) => [model.id, model]));
  const embedding = modelById.get('musicnn_embedding');
  if (!embedding) {
    return { status: 'model_missing', reason: '未下载 Essentia MusiCNN 特征模型' };
  }

  const { tf } = await getTensorflowRuntime();
  await configureTensorflowBackend(tf);
  const runtime = await getEssentiaRuntime();
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
  try {
    const embeddingModel = await loadTensorflowModel(tf, embedding);
    classifierModels.push(embeddingModel);
    const signal = audio.musicnnSignal;
    if (!signal) {
      throw new Error('MusiCNN 输入音频准备失败');
    }
    const features = await computeMusiCnnMelRows(
      runtime.essentia,
      signal,
      ({ processed, total }) => emitProgress({
        stage: 'extractingMusiCnn',
        message: `正在提取 MusiCNN 特征 ${processed}/${total}`,
        processed,
        total,
      }),
    );
    const patchSize = features.patchSize;
    const melBands = features.melBands;
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
    for (const { offset, validPatches } of musicCnnInferenceBatches(batchCount)) {
      const input = tf.tensor3d(
        paddedMel.subarray(offset * patchStride, (offset + validPatches) * patchStride),
        [validPatches, patchSize, melBands],
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
          for (const row of embeddingRows) {
            if (Array.isArray(row)) embeddingRowsFromMusicnn.push(row.map(Number));
          }
        }
        if (Array.isArray(tagRows)) {
          for (const row of tagRows) {
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
          runtime.essentia,
          signal,
          ({ processedPatches, totalPatches }) => emitProgress({
            stage: 'extractingDiscogs',
            modelFamily: 'discogsEffnet',
            modelId: 'discogs_effnet_embedding',
            message: `正在提取 Discogs-EffNet 特征 ${processedPatches}/${totalPatches}`,
            processed: processedPatches,
            total: totalPatches,
          }),
        );
        const embeddingStream = (async function* () {
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
                  validRows += 1;
                }
              }
              embeddingCount += validRows;
              if (validRows > 0) {
                yield { values: values.subarray(0, validRows * 1280), validRows };
              }
            } finally {
              discogsEmbeddingTensor?.dispose?.();
              discogsInput.dispose?.();
            }
          }
        })();
        const discogsRun = await runDiscogsEffnetHeadsStream(tf, embeddingStream, discogsHeadModels, {
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
    const audioBuffer = new ArrayBuffer(bytes.byteLength);
    new Uint8Array(audioBuffer).set(bytes);
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

async function prepareDecodedAudio(
  audio: AudioBuffer,
  includeMusicnnSignal: boolean,
): Promise<DecodedAudioData> {
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

export async function analyzeDecodedAudio(
  path: string,
  audio: DecodedAudioData,
  metadata?: TrackMetadata,
  options: {
    fingerprint?: AnalysisFingerprint;
    neteaseFilenameFormat?: NeteaseFilenameFormat;
    highLevel?: HighLevelAnalysis;
    highLevelModels?: EssentiaModelFile[];
    onProgress?: (progress: AnalysisWorkerProgress) => void;
  } = {},
): Promise<TrackAnalysis> {
  const neteaseFilenameFormat = options.neteaseFilenameFormat ?? 'title_artist';
  const resolvedMetadata = resolveTrackMetadata(path, metadata, neteaseFilenameFormat);
  const fallbackMetadata = filenameIdentity(path, neteaseFilenameFormat);
  const essentia = await getEssentia();
  const sampleRate = audio.sampleRate;
  const left = audio.channels[0] ?? new Float32Array();
  const right = audio.channels.length > 1 ? audio.channels[1] : left;
  const leftVector = essentia.arrayToVector(left);
  const rightVector = essentia.arrayToVector(right);
  let monoVector: any;

  try {
    const mixed = audio.channels.length > 1
      ? essentia.MonoMixer(leftVector, rightVector)
      : null;
    const mono = mixed ? essentia.vectorToArray(mixed.audio) : left;
    releaseVector(mixed?.audio);
    monoVector = essentia.arrayToVector(mono);
    const safe = <T>(operation: () => T): T | null => {
      try {
        return operation();
      } catch {
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
        highLevel = await runHighLevelAnalysis(audio, options.highLevelModels, options.onProgress);
      } catch (error) {
        // Basic analysis is useful on its own. An unexpected model/runtime
        // failure must not discard the values already computed above or turn
        // the output metadata write-back into an all-or-nothing operation.
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
    releaseVector(monoVector);
    releaseVector(leftVector);
    releaseVector(rightVector);
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
    workerClient?: AnalysisWorkerClientLike;
    workerJobId?: string;
    timeoutMs?: number;
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
  let decodedAudio: AudioBuffer | null = await resampleTo44100(
    await decodeAudio(bytes, Math.min(AUDIO_DECODE_TIMEOUT_MS, options.timeoutMs ?? AUDIO_DECODE_TIMEOUT_MS)),
  );
  const prepared = await prepareDecodedAudio(decodedAudio, Boolean(options.highLevelModels?.length));
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
