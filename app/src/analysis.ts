export type TrackAnalysis = {
  path: string;
  title: string;
  artist: string;
  album: string;
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
};

export type TrackMetadata = {
  title: string;
  artist: string;
  album: string;
};

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
  Energy: (audio: any) => any;
  Danceability: (audio: any) => any;
  delete: () => void;
};

type EssentiaConstructor = new (wasm: any, debug?: boolean) => EssentiaInstance;

let essentiaPromise: Promise<EssentiaInstance> | null = null;

async function getEssentia(): Promise<EssentiaInstance> {
  if (!essentiaPromise) {
    essentiaPromise = Promise.all([
      import('essentia.js/dist/essentia-wasm.es.js'),
      import('essentia.js/dist/essentia.js-extractor.es.js'),
    ]).then(([wasmModule, extractorModule]) => {
      const Constructor = extractorModule.default as unknown as EssentiaConstructor;
      return new Constructor(wasmModule.EssentiaWASM, false);
    });
  }
  return essentiaPromise;
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

function filenameIdentity(path: string): TrackMetadata {
  const filename = path.split(/[\\/]/).pop() || path;
  const stem = filename.replace(/\.[^.]+$/, '');
  const separator = stem.indexOf(' - ');
  if (separator > 0) {
    return {
      title: stem.slice(0, separator).trim(),
      artist: stem.slice(separator + 3).trim(),
      album: '',
    };
  }
  return { title: stem.trim(), artist: '', album: '' };
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
  metadata: TrackMetadata = filenameIdentity(path),
): Promise<TrackAnalysis> {
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
    return {
      path,
      title: metadata.title || filenameIdentity(path).title,
      artist: metadata.artist || filenameIdentity(path).artist,
      album: metadata.album,
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
      analysisVersion: '0.1.3',
    };
  } finally {
    releaseVector(monoVector);
    releaseVector(leftVector);
    releaseVector(rightVector);
  }
}
