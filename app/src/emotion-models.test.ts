import { describe, expect, it, vi } from 'vitest';
import { runEmotionHeads, MIREX_CLUSTER_LABELS, type EmotionModelId } from './emotion-models';
import type { EssentiaModelFile } from './analysis';

function model(id: EmotionModelId): EssentiaModelFile {
  return {
    id,
    modelJson: JSON.stringify({
      modelTopology: { id },
      weightsManifest: [{ weights: [] }],
    }),
    weightData: [1, 2, 3],
    classes: id === 'mirex' ? [...MIREX_CLUSTER_LABELS] : ['valence', 'arousal'],
    kind: id === 'mirex' ? 'emotionCluster' : 'emotionContinuous',
    outputName: id === 'mirex' ? 'PartitionedCall' : 'model/Identity',
    version: 'test',
  };
}

function fakeTf(outputs: Record<string, number[][]>, failures = new Set<string>()) {
  const disposed = { tensors: 0, models: 0 };
  const tf = {
    io: { fromMemory: (artifacts: { modelTopology: { id: string } }) => artifacts },
    loadGraphModel: vi.fn(async (artifacts: { modelTopology: { id: string } }) => {
      const id = artifacts.modelTopology.id;
      if (failures.has(id)) throw new Error(`${id} failed`);
      return {
        execute: () => ({
          array: async () => outputs[id],
          dispose: () => { disposed.tensors += 1; },
        }),
        dispose: () => { disposed.models += 1; },
      };
    }),
    tensor: vi.fn(() => ({ dispose: () => { disposed.tensors += 1; } })),
  };
  return { tf, disposed };
}

describe('independent emotion heads', () => {
  it('keeps continuous coordinates and MIREX clusters separate', async () => {
    const { tf, disposed } = fakeTf({
      emomusic: [[7, 3], [8, 5]],
      muse: [[4, 6], [6, 8]],
      mirex: [[0.1, 0.2, 0.3, 0.4, 0.5]],
    });
    const result = await runEmotionHeads(tf, [[0, 0]], new Map([
      ['emomusic', model('emomusic')],
      ['muse', model('muse')],
      ['mirex', model('mirex')],
    ]));

    expect(result.emotionCandidates.emomusic).toMatchObject({
      model: 'emomusic', status: 'completed', valence: 7.5, arousal: 4,
    });
    expect(result.emotionCandidates.muse).toMatchObject({
      model: 'muse', status: 'completed', valence: 5, arousal: 7,
    });
    expect(result.moodCluster).toHaveLength(5);
    expect(result.moodCluster[0]).toEqual({ label: 'passionate', confidence: 0.1 });
    expect(result.moodClusterStatus).toBe('completed');
    expect(result.failures).toEqual([]);
    expect(disposed.models).toBe(3);
  });

  it('marks missing and failed heads without clearing successful heads', async () => {
    const { tf } = fakeTf({ emomusic: [[7, 3]] }, new Set(['muse']));
    const result = await runEmotionHeads(tf, [[0, 0]], new Map([
      ['emomusic', model('emomusic')],
      ['muse', model('muse')],
    ]));

    expect(result.emotionCandidates.emomusic?.status).toBe('completed');
    expect(result.emotionCandidates.muse?.status).toBe('failed');
    expect(result.moodClusterStatus).toBe('model_missing');
    expect(result.failures.map((failure) => failure.model)).toEqual(['muse', 'mirex']);
  });

  it('does not create a successful result after cancellation', async () => {
    const { tf } = fakeTf({ emomusic: [[7, 3]], muse: [[4, 6]], mirex: [[0, 0, 0, 0, 0]] });
    const result = await runEmotionHeads(tf, [[0, 0]], new Map([
      ['emomusic', model('emomusic')],
      ['muse', model('muse')],
      ['mirex', model('mirex')],
    ]), { isCancelled: () => true });

    expect(result.emotionCandidates.emomusic?.status).toBe('cancelled');
    expect(result.emotionCandidates.muse?.status).toBe('cancelled');
    expect(result.moodClusterStatus).toBe('cancelled');
    expect(result.moodCluster).toEqual([]);
  });
});
