import {
  executeEssentiaModel,
  loadTensorflowModel,
  type AnalysisLabel,
  type DiscogsEffnetAnalysis,
  type DiscogsEffnetHeadId,
  type DiscogsEffnetHeadResult,
  type EssentiaModelFile,
} from './analysis';

export type TensorflowRuntime = {
  tensor2d: (values: ArrayLike<number> | number[] | number[][], shape?: [number, number], dtype?: string) => any;
};

export type DiscogsEmbeddingBatch = {
  values: Float32Array;
  validRows: number;
};

type HeadContract = {
  id: DiscogsEffnetHeadId;
  modelId: string;
  multiLabel: boolean;
  threshold?: number;
};

const HEAD_CONTRACTS: readonly HeadContract[] = [
  { id: 'moodTheme', modelId: 'discogs_mood_theme', multiLabel: true, threshold: 0.35 },
  { id: 'approachability', modelId: 'discogs_approachability', multiLabel: false },
  { id: 'instrumentation', modelId: 'discogs_instrumentation', multiLabel: true, threshold: 0.35 },
  { id: 'timbre', modelId: 'discogs_timbre', multiLabel: false },
  { id: 'danceability', modelId: 'discogs_danceability', multiLabel: false },
];

function meanScores(rows: unknown, classes: number): number[] {
  const list = Array.isArray(rows) ? rows : [];
  const normalized = list.length > 0 && Array.isArray(list[0]) ? list : [list];
  const totals = Array.from({ length: classes }, () => 0);
  let count = 0;
  for (const row of normalized) {
    if (!Array.isArray(row) || row.length < classes) continue;
    const values = row.slice(0, classes).map(Number);
    if (values.some((value) => !Number.isFinite(value))) continue;
    values.forEach((value, index) => { totals[index] += value; });
    count += 1;
  }
  return count > 0 ? totals.map((value) => value / count) : [];
}

export function aggregateDiscogsHead(
  contract: HeadContract,
  classes: readonly string[],
  predictions: unknown,
  frameCount: number,
  version: string,
): DiscogsEffnetHeadResult {
  const scoresArray = meanScores(predictions, classes.length);
  // Never put NaN into the wire object. JSON.stringify turns it into null,
  // while the Rust score map is numeric; an unavailable class is simply
  // omitted and the head's status/reason carries the diagnostic.
  const scores = Object.fromEntries(
    classes.flatMap((label, index) => {
      const score = scoresArray[index];
      return typeof score === 'number' && Number.isFinite(score) ? [[label, score]] : [];
    }),
  );
  const finiteLabels: AnalysisLabel[] = classes
    .map((label, index) => ({ label, confidence: scoresArray[index] ?? Number.NaN }))
    .filter((entry) => Number.isFinite(entry.confidence));
  if (contract.multiLabel) {
    const threshold = contract.threshold ?? 0.35;
    const labels = finiteLabels
      .filter((entry) => entry.confidence >= threshold)
      .sort((left, right) => right.confidence - left.confidence)
      .slice(0, 8);
    return {
      model: contract.id,
      status: 'completed',
      version,
      labels,
      scores,
      frameCount,
      threshold,
    };
  }
  const selected = finiteLabels.slice().sort((left, right) => right.confidence - left.confidence)[0];
  return {
    model: contract.id,
    status: 'completed',
    version,
    labels: selected ? [selected] : [],
    scores,
    frameCount,
    selectedClass: selected?.label,
    selectedConfidence: selected?.confidence,
  };
}

function missingResult(
  contract: HeadContract,
  frameCount: number,
  reason: string,
): DiscogsEffnetHeadResult {
  return {
    model: contract.id,
    status: 'model_missing',
    version: '',
    labels: [],
    scores: {},
    frameCount,
    ...(reason ? { reason } : {}),
  };
}

/** Run all five heads against one shared embedding matrix. */
export async function runDiscogsEffnetHeads(
  tf: TensorflowRuntime & Record<string, any>,
  embeddingRows: number[][],
  models: EssentiaModelFile[],
  options: {
    onProgress?: (modelId: DiscogsEffnetHeadId) => void;
    validRows?: number;
  } = {},
): Promise<DiscogsEffnetAnalysis> {
  const validRows = Math.max(0, Math.min(options.validRows ?? embeddingRows.length, embeddingRows.length));
  const rows = embeddingRows.slice(0, validRows).map((row) => row.slice(0, 1280));
  const heads: Partial<Record<DiscogsEffnetHeadId, DiscogsEffnetHeadResult>> = {};
  for (const contract of HEAD_CONTRACTS) {
    options.onProgress?.(contract.id);
    const model = models.find((candidate) => candidate.id === contract.modelId);
    if (!model) {
      heads[contract.id] = missingResult(contract, validRows, `未安装 ${contract.modelId}`);
      continue;
    }
    let classifier: any = null;
    let input: any = null;
    let outputTensor: any = null;
    try {
      classifier = await loadTensorflowModel(tf, model);
      input = tf.tensor2d(rows, [rows.length, 1280], 'float32');
      const output = executeEssentiaModel(tf, classifier, input, model.outputName);
      outputTensor = Array.isArray(output) ? output[0] : output;
      const predictions = await outputTensor.array();
      heads[contract.id] = aggregateDiscogsHead(
        contract,
        model.classes,
        predictions,
        validRows,
        model.version,
      );
    } catch (error) {
      heads[contract.id] = {
        model: contract.id,
        status: 'failed',
        version: model.version,
        labels: [],
        scores: {},
        frameCount: validRows,
        reason: error instanceof Error ? error.message : String(error),
      };
    } finally {
      outputTensor?.dispose?.();
      input?.dispose?.();
      classifier?.dispose?.();
    }
  }
  return {
    embeddingModel: 'discogs-effnet-bs64-1',
    embeddingDimensions: 1280,
    inputShape: [64, 128, 96],
    heads,
  };
}

/** Run the five heads over streamed embedding batches. Models are loaded once
 * and each input/output tensor is released before the next batch arrives. */
export async function runDiscogsEffnetHeadsStream(
  tf: TensorflowRuntime & Record<string, any>,
  batches: AsyncIterable<DiscogsEmbeddingBatch>,
  models: EssentiaModelFile[],
  options: {
    onProgress?: (modelId: DiscogsEffnetHeadId) => void;
  } = {},
): Promise<DiscogsEffnetAnalysis> {
  const heads: Partial<Record<DiscogsEffnetHeadId, DiscogsEffnetHeadResult>> = {};
  const totalRowsByHead = new Map<DiscogsEffnetHeadId, number>();
  const sumsByHead = new Map<DiscogsEffnetHeadId, number[]>();
  const versions = new Map<DiscogsEffnetHeadId, string>();
  const modelsByHead = new Map<DiscogsEffnetHeadId, { spec: EssentiaModelFile; model: any }>();

  for (const contract of HEAD_CONTRACTS) {
    options.onProgress?.(contract.id);
    const spec = models.find((candidate) => candidate.id === contract.modelId);
    if (!spec) {
      heads[contract.id] = missingResult(contract, 0, `未安装 ${contract.modelId}`);
      continue;
    }
    try {
      modelsByHead.set(contract.id, {
        spec,
        model: await loadTensorflowModel(tf, spec),
      });
      versions.set(contract.id, spec.version);
    } catch (error) {
      heads[contract.id] = {
        model: contract.id,
        status: 'failed',
        version: spec.version,
        labels: [],
        scores: {},
        frameCount: 0,
        reason: error instanceof Error ? error.message : String(error),
      };
    }
  }

  try {
    for await (const batch of batches) {
      const validRows = Math.max(0, Math.min(batch.validRows, Math.floor(batch.values.length / 1280)));
      if (validRows === 0) continue;
      for (const contract of HEAD_CONTRACTS) {
        const entry = modelsByHead.get(contract.id);
        if (!entry) {
          continue;
        }
        let input: any = null;
        let outputTensor: any = null;
        try {
          input = tf.tensor2d(
            batch.values.subarray(0, validRows * 1280),
            [validRows, 1280],
            'float32',
          );
          const output = executeEssentiaModel(tf, entry.model, input, entry.spec.outputName);
          outputTensor = Array.isArray(output) ? output[0] : output;
          const predictions = await outputTensor.array();
          const scores = meanScores(predictions, entry.spec.classes.length);
          if (scores.length > 0) {
            const totals = sumsByHead.get(contract.id)
              ?? Array.from({ length: entry.spec.classes.length }, () => 0);
            scores.forEach((value, index) => { totals[index] += value * validRows; });
            sumsByHead.set(contract.id, totals);
            totalRowsByHead.set(contract.id, (totalRowsByHead.get(contract.id) ?? 0) + validRows);
          }
        } catch (error) {
          heads[contract.id] = {
            model: contract.id,
            status: 'failed',
            version: entry.spec.version,
            labels: [],
            scores: {},
            frameCount: totalRowsByHead.get(contract.id) ?? 0,
            reason: error instanceof Error ? error.message : String(error),
          };
        } finally {
          outputTensor?.dispose?.();
          input?.dispose?.();
        }
      }
    }

    for (const contract of HEAD_CONTRACTS) {
      if (heads[contract.id]?.status === 'failed' || !modelsByHead.has(contract.id)) continue;
      const entry = modelsByHead.get(contract.id);
      const count = totalRowsByHead.get(contract.id) ?? 0;
      if (!entry || count === 0) {
        heads[contract.id] = missingResult(contract, 0, 'Discogs-EffNet 未返回有效嵌入');
        continue;
      }
      const totals = sumsByHead.get(contract.id) ?? [];
      const mean = totals.map((value) => value / count);
      heads[contract.id] = aggregateDiscogsHead(
        contract,
        entry.spec.classes,
        [mean],
        count,
        versions.get(contract.id) ?? entry.spec.version,
      );
    }
  } finally {
    for (const { model } of modelsByHead.values()) model?.dispose?.();
  }

  return {
    embeddingModel: 'discogs-effnet-bs64-1',
    embeddingDimensions: 1280,
    inputShape: [64, 128, 96],
    heads,
  };
}

export const discogsEffnetHeadContracts = HEAD_CONTRACTS;
