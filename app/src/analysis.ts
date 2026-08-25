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

export type HighLevelAnalysis = {
  status: 'completed' | 'model_missing' | 'failed';
  modelVersion?: string | null;
  reason?: string | null;
  genre?: AnalysisLabel[];
  mood?: AnalysisLabel[];
  instrument?: AnalysisLabel[];
  filtered?: Array<{ label: string; confidence: number; reason: string }>;
};

export type EssentiaModelFile = {
  id: string;
  modelJson: string;
  weightData: number[];
  classes: string[];
  kind: 'embedding' | 'genre' | 'mood' | 'instrument';
  version: string;
};

export const ESSENTIA_MODEL_IDS = [
  'musicnn_embedding',
  'genre_rosamerica',
  'mood_aggressive',
  'mood_happy',
  'mood_relaxed',
  'mood_party',
  'mood_sad',
  'voice_instrumental',
] as const;

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

type EssentiaInstance = {
  arrayToVector: (input: Float32Array) => any;
  vectorToArray: (input: any) => Float32Array;
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
};

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
): { accepted: AnalysisLabel[]; filtered: Array<{ label: string; confidence: number; reason: string }> } {
  const accepted: AnalysisLabel[] = [];
  const filtered: Array<{ label: string; confidence: number; reason: string }> = [];
  for (const label of labels) {
    const normalized = label.label.trim().toLowerCase();
    if (NEGATIVE_HIGH_LEVEL_LABELS.has(normalized)) {
      filtered.push({ ...label, reason: 'negative_label' });
    } else if (!Number.isFinite(label.confidence) || label.confidence < threshold) {
      filtered.push({ ...label, reason: 'below_threshold' });
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
      return {
        essentia: new Constructor(wasmModule.EssentiaWASM, false),
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

async function loadTensorflowModel(tf: any, model: EssentiaModelFile): Promise<any> {
  const artifacts = modelArtifacts(model);
  const weightData = Uint8Array.from(model.weightData).buffer;
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

function executeEssentiaModel(
  tf: any,
  model: any,
  featureTensor: any,
  outputName?: string,
): any {
  // Essentia's TensorflowMusiCNN wrapper puts optional inputs before the
  // feature tensor: [isTraining, features] for the embedding model. Keep
  // that ordering here instead of relying on the topology's node order.
  const inputCount = model?.executor?.inputs?.length ?? model?.inputs?.length ?? 1;
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
}

async function runHighLevelAnalysis(
  audio: AudioBuffer,
  models: EssentiaModelFile[],
): Promise<HighLevelAnalysis> {
  const modelById = new Map(models.map((model) => [model.id, model]));
  const embedding = modelById.get('musicnn_embedding');
  if (!embedding) {
    return { status: 'model_missing', reason: '未下载 Essentia MusiCNN 特征模型' };
  }

  const { tf, InputExtractor } = await getTensorflowRuntime();
  const runtime = await getEssentiaRuntime();
  const extractor = new InputExtractor(runtime.wasmModule.EssentiaWASM, 'musicnn', false);
  const classifierModels: any[] = [];
  try {
    const embeddingModel = await loadTensorflowModel(tf, embedding);
    classifierModels.push(embeddingModel);
    const signal = await extractor.downsampleAudioBuffer(audio);
    const features = extractor.computeFrameWise(signal, 256);
    const melRows = Array.isArray(features.melSpectrum) ? features.melSpectrum as number[][] : [];
    const patchSize = Number(features.patchSize) || 187;
    const melBands = Number(features.melBandsSize) || 96;
    const batchCount = Math.max(1, Math.ceil(melRows.length / patchSize));
    const paddedMel = Array.from({ length: batchCount * patchSize }, (_, index) =>
      melRows[index] ?? Array.from({ length: melBands }, () => 0),
    );
    const input = tf.tensor(paddedMel, [batchCount, patchSize, melBands], 'float32');
    const output = executeEssentiaModel(tf, embeddingModel, input, 'model/dense/BiasAdd');
    const tensor = Array.isArray(output) ? output[0] : output;
    const embeddings = await tensor.array();
    input.dispose();
    tensor.dispose();

    const filtered: Array<{ label: string; confidence: number; reason: string }> = [];
    const genre: AnalysisLabel[] = [];
    const mood: AnalysisLabel[] = [];
    const instrument: AnalysisLabel[] = [];
    for (const model of models.filter((candidate) => candidate.kind !== 'embedding')) {
      const classifier = await loadTensorflowModel(tf, model);
      classifierModels.push(classifier);
      const embeddingRows = Array.isArray(embeddings) ? embeddings : [];
      const input = tf.tensor(embeddingRows, [embeddingRows.length, 200], 'float32');
      const output = executeEssentiaModel(tf, classifier, input);
      const tensor = Array.isArray(output) ? output[0] : output;
      const predictions = await tensor.array();
      input.dispose();
      tensor.dispose();
      const scores = averagePredictions(predictions, model.classes.length);
      const labels = model.classes.map((label, index) => ({
        label,
        confidence: scores[index] ?? Number.NaN,
      }));
      if (model.kind === 'genre') {
        const top = labels.reduce<AnalysisLabel | null>(
          (best, label) => !best || label.confidence > best.confidence ? label : best,
          null,
        );
        if (top) {
          const result = filterHighLevelLabels([top]);
          genre.push(...result.accepted);
          filtered.push(...result.filtered);
        }
      } else if (model.kind === 'mood') {
        const result = filterHighLevelLabels(labels);
        mood.push(...result.accepted);
        filtered.push(...result.filtered);
      } else {
        const result = filterHighLevelLabels(labels);
        instrument.push(...result.accepted);
        filtered.push(...result.filtered);
      }
    }
    return {
      status: 'completed',
      modelVersion: embedding.version,
      genre,
      mood,
      instrument,
      filtered,
    };
  } catch (error) {
    return {
      status: 'failed',
      modelVersion: embedding.version,
      reason: error instanceof Error ? error.message : String(error),
    };
  } finally {
    extractor.delete();
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

async function decodeAudio(bytes: Uint8Array): Promise<AudioBuffer> {
  const AudioContextConstructor = window.AudioContext ||
    (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioContextConstructor) {
    throw new Error('当前系统不支持 Web Audio 音频解码');
  }

  const context = new AudioContextConstructor();
  try {
    const audioBuffer = new ArrayBuffer(bytes.byteLength);
    new Uint8Array(audioBuffer).set(bytes);
    return await context.decodeAudioData(audioBuffer);
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

export async function analyzeAudioFile(
  path: string,
  bytes: Uint8Array,
  metadata?: TrackMetadata,
  options: {
    fingerprint?: AnalysisFingerprint;
    neteaseFilenameFormat?: NeteaseFilenameFormat;
    highLevel?: HighLevelAnalysis;
    highLevelModels?: EssentiaModelFile[];
  } = {},
): Promise<TrackAnalysis> {
  const neteaseFilenameFormat = options.neteaseFilenameFormat ?? 'title_artist';
  const resolvedMetadata = resolveTrackMetadata(path, metadata, neteaseFilenameFormat);
  const fallbackMetadata = filenameIdentity(path, neteaseFilenameFormat);
  const audio = await resampleTo44100(await decodeAudio(bytes));
  const essentia = await getEssentia();
  const sampleRate = 44100;
  const left = audio.getChannelData(0);
  const right = audio.numberOfChannels > 1 ? audio.getChannelData(1) : left;
  const leftVector = essentia.arrayToVector(left);
  const rightVector = essentia.arrayToVector(right);
  let monoVector: any;

  try {
    const mixed = audio.numberOfChannels > 1
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
      highLevel = await runHighLevelAnalysis(audio, options.highLevelModels);
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
