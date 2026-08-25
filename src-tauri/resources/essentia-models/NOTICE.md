# Bundled Essentia models

W4DJ bundles the MSD MusiCNN feature extractor plus mood, voice/instrument,
emotion-head, and Discogs style classification resources created by the Music
Technology Group (MTG), Universitat Pompeu Fabra. The official `emomusic`,
`muse`, and `moods_mirex` ONNX exports are converted offline into equivalent
TensorFlow.js graphs; weights are copied without retraining or fabrication.
The Discogs EffNet graph is normalized from the official frozen graph by
inlining its TensorFlow function library because the browser GraphModel
executor does not provide a generic `PartitionedCall` kernel. This preserves
the official operations and constants and does not add a network fallback.

The model files are distributed under the Creative Commons Attribution-NonCommercial-ShareAlike 4.0 license. The complete license terms are available at <https://creativecommons.org/licenses/by-nc-sa/4.0/legalcode>. Commercial licensing is available from MTG at <https://www.upf.edu/web/mtg/contact>.

Official model catalog: <https://essentia.upf.edu/models/>

Included resources:

- `musicnn_embedding.*`: MSD MusiCNN TensorFlow.js model. W4DJ uses its 200-dimensional embedding and 50-tag outputs.
- `mood_*.{json,bin}` and `voice_instrumental.{json,bin}`: official Essentia classification-head weights converted from the corresponding TensorFlow/ONNX exports into an equivalent TensorFlow.js graph layout. The weights are unchanged; filenames and graph serialization are normalized for W4DJ's local loader.
- `emomusic.{json,bin}`, `muse.{json,bin}`, and `mirex.{json,bin}`: official Essentia emotion heads converted from ONNX and validated offline.
- `discogs_effnet_embedding.{json,bin}`: official Discogs EffNet embedding model, accepting `[64,128,96]` mel patches and returning 1280-dimensional embeddings. The legacy `discogs_effnet.{json,bin}` pair is retained only for backward-compatible imports.
- `discogs_mood_theme.{json,bin}`, `discogs_approachability.{json,bin}`, `discogs_instrumentation.{json,bin}`, `discogs_timbre.{json,bin}`, and `discogs_danceability.{json,bin}`: official Discogs EffNet classification heads operating on the shared 1280-dimensional embedding. Their outputs are stored in W4DJ namespaced metadata fields and are independent of the legacy scalar `W4DJ-Danceability` field.
- `genre_discogs400.{json,bin}` and `genre_discogs400.labels.json`: official 400-class Discogs genre head and its class labels. Genre output remains a separate optional projection.

These normalized files are an adaptation under the same CC BY-NC-SA 4.0 terms. Source preparation is documented in `scripts/prepare_essentia_tfjs_resources.py` in the W4DJ source tree.

For reproducibility, the Discogs graphs were converted offline with the pinned
TensorFlow.js converter environment, then normalized with:

```text
python3 scripts/prepare_essentia_tfjs_resources.py \
  --embedding-json <official-msd-musicnn-1-tfjs>/model.json \
  --embedding-bin <official-msd-musicnn-1-tfjs>/group1-shard1of1.bin \
  --onnx-dir <verified-musicnn-heads> \
  --emotion-onnx-dir <verified-emotion-heads> \
  --output src-tauri/resources/essentia-models \
  --discogs-embedding-dir <discogs-effnet-bs64-1-tfjs> \
  --discogs-genre-dir <genre-discogs400-tfjs> \
  --discogs-heads-dir <discogs-heads-tfjs>
```

The command is a developer/offline preparation step only. The desktop app
loads these local pairs and never downloads model files at startup or during
analysis.

The TensorFlow.js runtime bundled by W4DJ is licensed under Apache-2.0. Essentia.js is licensed under AGPL-3.0; see each dependency's package metadata and license file in the source distribution.
