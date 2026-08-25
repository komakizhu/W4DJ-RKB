import type {
  AnalysisLabel,
  ContinuousEmotionResult,
  EmotionCandidates,
  EmotionHeadStatus,
  EssentiaModelFile,
} from './analysis';
import { modelWeightDataBuffer } from './analysis';

export type EmotionModelId = 'emomusic' | 'muse' | 'mirex';

export type EmotionRunOptions = {
  isCancelled?: () => boolean;
  onProgress?: (model: EmotionModelId) => void;
};

export type EmotionHeadRun = {
  emotionCandidates: EmotionCandidates;
  moodCluster: AnalysisLabel[];
  moodClusterStatus: EmotionHeadStatus;
  moodClusterReason?: string;
  failures: Array<{ model: EmotionModelId; reason: string }>;
};

export const MIREX_CLUSTER_LABELS = [
  'passionate',
  'rollicking',
  'literate',
  'humorous',
  'aggressive',
] as const;

const CONTINUOUS_MODEL_IDS = ['emomusic', 'muse'] as const;

function modelArtifacts(model: EssentiaModelFile): {
  modelTopology: unknown;
  weightSpecs: unknown[];
} {
  const parsed = JSON.parse(model.modelJson) as {
    modelTopology?: unknown;
    weightsManifest?: Array<{ weights?: unknown[] }>;
  };
  if (!parsed.modelTopology || !parsed.weightsManifest) {
    throw new Error(`情绪模型 ${model.id} 的结构不完整`);
  }
  return {
    modelTopology: parsed.modelTopology,
    weightSpecs: parsed.weightsManifest.flatMap((manifest) => manifest.weights ?? []),
  };
}

async function loadEmotionModel(tf: any, model: EssentiaModelFile): Promise<any> {
  const artifacts = modelArtifacts(model);
  const weightData = modelWeightDataBuffer(model.weightData);
  return tf.loadGraphModel(tf.io.fromMemory({
    modelTopology: artifacts.modelTopology,
    weightSpecs: artifacts.weightSpecs,
    weightData,
  }));
}

function executeModel(tf: any, model: any, input: any, outputName?: string): any {
  const run = () => {
    const output = outputName ? model.execute(input, outputName) : model.execute(input);
    return Array.isArray(output) ? output[0] : output;
  };
  return typeof tf.tidy === 'function' ? tf.tidy(run) : run();
}

function emotionInputTensor(tf: any, model: any, embeddingRows: readonly number[][]): any {
  const shape = model?.inputs?.[0]?.shape;
  // The official Essentia TFJS directories for the v1 heads expose
  // [-1, 1, 200], while the v2 ONNX->TFJS resources bundled by W4DJ use
  // [-1, 200].  Honour the graph's declared rank so imported official pairs
  // and bundled pairs share the same execution path.
  if (Array.isArray(shape) && shape.length === 3 && shape[2] === 200) {
    const middle = Number(shape[1]);
    if (middle === 1 || !Number.isFinite(middle) || middle < 0) {
      return tf.tensor(embeddingRows, [embeddingRows.length, 1, 200], 'float32');
    }
  }
  return tf.tensor(embeddingRows, [embeddingRows.length, 200], 'float32');
}

function predictionRows(predictions: unknown): number[][] {
  if (!Array.isArray(predictions)) return [];
  if (predictions.length === 0) return [];
  if (Array.isArray(predictions[0])) {
    return predictions.filter((row): row is unknown[] => Array.isArray(row))
      .map((row) => row.map((value) => Number(value)));
  }
  return [predictions.map((value) => Number(value))];
}

function averageColumns(predictions: unknown, columnCount: number): number[] {
  const rows = predictionRows(predictions);
  const totals = Array.from({ length: columnCount }, () => 0);
  let count = 0;
  for (const row of rows) {
    if (row.length < columnCount || row.slice(0, columnCount).some((value) => !Number.isFinite(value))) {
      continue;
    }
    row.slice(0, columnCount).forEach((value, index) => {
      totals[index] += value;
    });
    count += 1;
  }
  return count > 0 ? totals.map((value) => value / count) : [];
}

function continuousResult(
  id: 'emomusic' | 'muse',
  values: number[],
): ContinuousEmotionResult {
  if (values.length < 2 || values.slice(0, 2).some((value) => !Number.isFinite(value) || value < 1 || value > 9)) {
    throw new Error(`${id} 输出不是有限的 1–9 Valence/Arousal 坐标`);
  }
  return {
    model: id,
    status: 'completed',
    valence: values[0],
    arousal: values[1],
    reason: null,
  };
}

function missingContinuous(id: 'emomusic' | 'muse'): ContinuousEmotionResult {
  return {
    model: id,
    status: 'model_missing',
    valence: null,
    arousal: null,
    reason: '模型资源未安装或未通过严格校验',
  };
}

function cancelledContinuous(id: 'emomusic' | 'muse'): ContinuousEmotionResult {
  return {
    model: id,
    status: 'cancelled',
    valence: null,
    arousal: null,
    reason: '分析已取消',
  };
}

function failedContinuous(id: 'emomusic' | 'muse', reason: string): ContinuousEmotionResult {
  return {
    model: id,
    status: 'failed',
    valence: null,
    arousal: null,
    reason,
  };
}

export async function runEmotionHeads(
  tf: any,
  embeddingRows: readonly number[][],
  models: ReadonlyMap<string, EssentiaModelFile>,
  options: EmotionRunOptions = {},
): Promise<EmotionHeadRun> {
  const emotionCandidates: EmotionCandidates = {};
  const failures: Array<{ model: EmotionModelId; reason: string }> = [];
  let moodCluster: AnalysisLabel[] = [];
  let moodClusterStatus: EmotionHeadStatus = 'model_missing';
  let moodClusterReason: string | undefined = '模型资源未安装或未通过严格校验';

  const markCancelled = () => {
    CONTINUOUS_MODEL_IDS.forEach((id) => {
      if (!emotionCandidates[id]) emotionCandidates[id] = cancelledContinuous(id);
    });
    if (moodClusterStatus === 'model_missing') {
      moodClusterStatus = 'cancelled';
      moodClusterReason = '分析已取消';
    }
  };

  if (options.isCancelled?.()) {
    markCancelled();
    return { emotionCandidates, moodCluster, moodClusterStatus, moodClusterReason, failures };
  }

  const runContinuous = async (id: 'emomusic' | 'muse'): Promise<void> => {
    const model = models.get(id);
    if (!model) {
      emotionCandidates[id] = missingContinuous(id);
      failures.push({ model: id, reason: emotionCandidates[id].reason || 'model_missing' });
      return;
    }
    if (options.isCancelled?.()) {
      emotionCandidates[id] = cancelledContinuous(id);
      return;
    }
    options.onProgress?.(id);
    let loaded: any = null;
    let input: any = null;
    let output: any = null;
    try {
      loaded = await loadEmotionModel(tf, model);
      input = emotionInputTensor(tf, loaded, embeddingRows);
      output = executeModel(tf, loaded, input, model.outputName || 'model/Identity');
      const values = averageColumns(await output.array(), 2);
      if (options.isCancelled?.()) {
        emotionCandidates[id] = cancelledContinuous(id);
      } else {
        emotionCandidates[id] = continuousResult(id, values);
      }
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      emotionCandidates[id] = options.isCancelled?.()
        ? cancelledContinuous(id)
        : failedContinuous(id, reason);
      if (!options.isCancelled?.()) failures.push({ model: id, reason });
    } finally {
      output?.dispose?.();
      input?.dispose?.();
      loaded?.dispose?.();
    }
  };

  for (const id of CONTINUOUS_MODEL_IDS) {
    await runContinuous(id);
    if (options.isCancelled?.()) {
      markCancelled();
      return { emotionCandidates, moodCluster, moodClusterStatus, moodClusterReason, failures };
    }
  }

  const mirex = models.get('mirex');
  if (!mirex) {
    failures.push({ model: 'mirex', reason: moodClusterReason });
    return { emotionCandidates, moodCluster, moodClusterStatus, moodClusterReason, failures };
  }
  if (options.isCancelled?.()) {
    markCancelled();
    return { emotionCandidates, moodCluster, moodClusterStatus, moodClusterReason, failures };
  }
  options.onProgress?.('mirex');
  let loaded: any = null;
  let input: any = null;
  let output: any = null;
  try {
    loaded = await loadEmotionModel(tf, mirex);
    input = emotionInputTensor(tf, loaded, embeddingRows);
    output = executeModel(tf, loaded, input, mirex.outputName || 'PartitionedCall');
    const values = averageColumns(await output.array(), MIREX_CLUSTER_LABELS.length);
    if (values.length < MIREX_CLUSTER_LABELS.length || values.some((value) => !Number.isFinite(value))) {
      throw new Error('MIREX 输出不是五个有限的情绪簇置信度');
    }
    if (options.isCancelled?.()) {
      moodClusterStatus = 'cancelled';
      moodClusterReason = '分析已取消';
    } else {
      moodCluster = MIREX_CLUSTER_LABELS.map((label, index) => ({
        label,
        confidence: values[index],
      }));
      moodClusterStatus = 'completed';
      moodClusterReason = undefined;
    }
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error);
    moodClusterStatus = options.isCancelled?.() ? 'cancelled' : 'failed';
    moodClusterReason = options.isCancelled?.() ? '分析已取消' : reason;
    if (!options.isCancelled?.()) failures.push({ model: 'mirex', reason });
  } finally {
    output?.dispose?.();
    input?.dispose?.();
    loaded?.dispose?.();
  }

  return { emotionCandidates, moodCluster, moodClusterStatus, moodClusterReason, failures };
}
