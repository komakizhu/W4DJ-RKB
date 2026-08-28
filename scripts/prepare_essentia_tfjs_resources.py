#!/usr/bin/env python3
"""Prepare bundled Essentia TensorFlow.js files from verified local sources.

This developer utility is intentionally not part of the app runtime. It expects
the official MusiCNN TensorFlow.js pair plus ONNX exports of the small
classification heads and writes the normalized resource names used by W4DJ.
"""

from __future__ import annotations

import argparse
import os
import json
import shutil
import tempfile
from pathlib import Path

try:
    import onnx
except ModuleNotFoundError:  # ``--help`` should work without converter extras.
    onnx = None


def require_onnx():
    if onnx is None:
        raise RuntimeError("ONNX conversion requires the pinned converter environment")
    return onnx


HEAD_IDS = (
    "mood_aggressive",
    "mood_happy",
    "mood_relaxed",
    "mood_party",
    "mood_sad",
    "voice_instrumental",
)

EMOTION_HEADS = (
    ("emomusic", "emomusic-msd-musicnn-2.onnx", 2, "model/Identity"),
    ("muse", "muse-msd-musicnn-2.onnx", 2, "model/Identity"),
    ("mirex", "moods_mirex-msd-musicnn-1.onnx", 5, "PartitionedCall"),
)

DISCOGS_HEADS = (
    ("discogs_effnet_embedding", "discogs_embedding", 1280, 1),
    ("genre_discogs400", "discogs_genre", 400, 0),
)

# The legacy identifier remains accepted by the runtime importer.  This name
# is used here only to remove an exact duplicate left by older resource
# preparation runs when an existing output directory is reused.
LEGACY_DISCOGS_EMBEDDING_ID = "discogs_effnet"

DISCOGS_EXTRA_HEADS = (
    ("discogs_mood_theme", "moodtheme", "model/Sigmoid", 56, 1280),
    ("discogs_approachability", "approachability", "model/Softmax", 2, 1280),
    ("discogs_instrumentation", "instrumentation", "model/Sigmoid", 40, 1280),
    ("discogs_timbre", "timbre", "model/Softmax", 2, 1280),
    ("discogs_danceability", "danceability", "model/Softmax", 2, 1280),
)


def tensor_attribute(dtype: str, shape: list[int]) -> dict:
    return {
        "dtype": {"type": dtype},
        "value": {"tensor": {"dtype": dtype, "tensorShape": {
            "dim": [{"size": str(size)} for size in shape]
        }}},
    }


def graph_model(units: int, output_name: str = "model/Softmax") -> dict:
    float_attr = {"T": {"type": "DT_FLOAT"}}
    nodes = [
        {"name": "model/Placeholder", "op": "Placeholder", "attr": {
            "dtype": {"type": "DT_FLOAT"},
            "shape": {"shape": {"dim": [{"size": "-1"}, {"size": "200"}]}},
        }},
        {"name": "dense/kernel", "op": "Const", "attr": tensor_attribute("DT_FLOAT", [200, 100])},
        {"name": "dense/bias", "op": "Const", "attr": tensor_attribute("DT_FLOAT", [100])},
        {"name": "dense_1/kernel", "op": "Const", "attr": tensor_attribute("DT_FLOAT", [100, units])},
        {"name": "dense_1/bias", "op": "Const", "attr": tensor_attribute("DT_FLOAT", [units])},
        {"name": "model/dense/MatMul", "op": "MatMul", "input": ["model/Placeholder", "dense/kernel"], "attr": {
            **float_attr, "transpose_a": {"b": False}, "transpose_b": {"b": False},
        }},
        {"name": "model/dense/BiasAdd", "op": "BiasAdd", "input": ["model/dense/MatMul", "dense/bias"], "attr": float_attr},
        {"name": "model/dense/Relu", "op": "Relu", "input": ["model/dense/BiasAdd"], "attr": float_attr},
        {"name": "model/dense_1/MatMul", "op": "MatMul", "input": ["model/dense/Relu", "dense_1/kernel"], "attr": {
            **float_attr, "transpose_a": {"b": False}, "transpose_b": {"b": False},
        }},
        {"name": "model/dense_1/BiasAdd", "op": "BiasAdd", "input": ["model/dense_1/MatMul", "dense_1/bias"], "attr": float_attr},
        {"name": output_name, "op": "Identity" if output_name == "model/Identity" else "PartitionedCall" if output_name == "PartitionedCall" else "Softmax", "input": ["model/dense_1/BiasAdd"], "attr": float_attr},
    ]
    weights = [
        {"name": "dense/kernel", "shape": [200, 100], "dtype": "float32"},
        {"name": "dense_1/bias", "shape": [units], "dtype": "float32"},
        {"name": "dense/bias", "shape": [100], "dtype": "float32"},
        {"name": "dense_1/kernel", "shape": [100, units], "dtype": "float32"},
    ]
    return {
        "format": "graph-model",
        "generatedBy": "W4DJ resource preparation from Essentia ONNX export",
        "convertedBy": "W4DJ",
        "signature": {"outputs": {output_name: {"name": output_name}}},
        "modelTopology": {"node": nodes, "library": {}, "versions": {}},
        "weightsManifest": [{"paths": ["group1-shard1of1.bin"], "weights": weights}],
    }


def canonical_tensor_name(name: str) -> str:
    """Use the graph-node spelling TensorFlow.js expects for a tensor name."""
    return name[:-2] if name.endswith(":0") else name


def onnx_tensor_attribute(shape: list[int]) -> dict:
    return tensor_attribute("DT_FLOAT", shape)


def _tfjs_ref_base(value: str) -> str:
    """Return the graph-node part of a TensorFlow tensor/control ref."""
    value = value.lstrip("^")
    return value.split(":", 1)[0]


def inline_tfjs_function_graph(value: dict, output_name: str, output_index: int) -> None:
    """Inline converter-emitted PartitionedCall functions for TFJS GraphModel.

    The TensorFlow.js converter preserves TensorFlow 2 ``PartitionedCall``
    nodes and their function library.  The browser GraphModel executor used by
    W4DJ intentionally has no generic PartitionedCall kernel, so the official
    frozen graph is normalized into the equivalent ordinary node graph.  This
    keeps the original constants and operations; it does not retrain,
    approximate, or replace the official model.
    """
    topology = value.get("modelTopology") or {}
    top_nodes = list(topology.get("node") or [])
    call = next((node for node in top_nodes if node.get("op") == "PartitionedCall"), None)
    if call is None:
        raise ValueError("Discogs graph has no PartitionedCall output")
    functions = {
        function.get("signature", {}).get("name"): function
        for function in topology.get("library", {}).get("function", [])
        if function.get("signature", {}).get("name")
    }
    emitted = [node for node in top_nodes if node is not call]
    emitted_names = {node.get("name") for node in emitted}

    def resolve(reference: str, aliases: dict[str, str]) -> str:
        base = _tfjs_ref_base(reference)
        return aliases.get(reference, aliases.get(base, base))

    def alias_node(aliases: dict[str, str], name: str, value_ref: str) -> None:
        aliases[name] = value_ref
        aliases[f"{name}:0"] = value_ref
        aliases[f"{name}:output:0"] = value_ref
        aliases[f"{name}:product:0"] = value_ref
        aliases[f"{name}:z:0"] = value_ref
        aliases[f"{name}:y:0"] = value_ref

    def inline_function(function_name: str, external_inputs: list[str]) -> dict[str, str]:
        function = functions.get(function_name)
        if function is None:
            raise ValueError(f"Discogs graph references missing function {function_name}")
        signature = function.get("signature") or {}
        input_args = [argument.get("name") for argument in signature.get("inputArg", [])]
        if len(input_args) != len(external_inputs):
            raise ValueError(
                f"Discogs function {function_name} expects {len(input_args)} inputs, "
                f"got {len(external_inputs)}"
            )
        aliases = dict(zip(input_args, external_inputs))
        for name, external in list(aliases.items()):
            alias_node(aliases, name, external)
        for node in function.get("nodeDef", []):
            name = node.get("name")
            op = node.get("op")
            if not name or not op:
                raise ValueError(f"Discogs function {function_name} contains an invalid node")
            inputs = [resolve(item, aliases) for item in node.get("input", [])]
            if op == "PartitionedCall":
                nested_name = (((node.get("attr") or {}).get("f") or {}).get("func") or {}).get("name")
                if not nested_name:
                    raise ValueError("Discogs PartitionedCall has no function name")
                nested = inline_function(nested_name, inputs)
                nested_values = list(nested.values())
                alias_node(aliases, name, nested_values[0] if nested_values else name)
                for index, nested_value in enumerate(nested_values):
                    aliases[f"{name}:output:{index}"] = nested_value
                    aliases[f"{name}:{index}"] = nested_value
                continue
            cloned = dict(node)
            if inputs:
                cloned["input"] = inputs
            if name in emitted_names:
                raise ValueError(f"Discogs graph contains duplicate node {name}")
            emitted.append(cloned)
            emitted_names.add(name)
            alias_node(aliases, name, name)
        return {
            key: resolve(reference, aliases)
            for key, reference in (function.get("ret") or {}).items()
        }

    call_inputs = list(call.get("input") or [])
    function_name = ((((call.get("attr") or {}).get("f") or {}).get("func") or {}).get("name"))
    if not function_name:
        raise ValueError("Discogs output call has no function name")
    returns = inline_function(function_name, call_inputs)
    return_values = list(returns.values())
    if output_index < 0 or output_index >= len(return_values):
        raise ValueError(f"Discogs output index {output_index} is unavailable")
    source = return_values[output_index]
    emitted.append({
        "name": output_name,
        "op": "Identity",
        "input": [source],
        "attr": {
            "T": {"type": "DT_FLOAT"},
            "_output_shapes": {"list": {"shape": [{"dim": [
                {"size": "64" if output_name == "discogs_embedding" else "-1"},
                {"size": "1280" if output_name == "discogs_embedding" else "400"},
            ]}]}}
        },
    })
    topology["node"] = emitted
    topology["library"] = {}
    value["modelTopology"] = topology
    value["signature"] = {"outputs": {output_name: {"name": output_name}}}


def convert_mirex(source: Path, destination: Path, model_id: str, expected_units: int) -> None:
    """Convert the official MIREX MusicNN ONNX head without changing its graph.

    The MIREX head has a batch-normalisation layer and a second dense layer,
    unlike the small two-layer emotion heads.  Re-creating the graph from its
    ONNX nodes keeps the official weights and operations intact while still
    producing the graph-model format used by the application.
    """
    model = require_onnx().load(source)
    if len(model.graph.input) != 1 or len(model.graph.output) != 1:
        raise ValueError(f"{source.name} must have one input and one output")
    input_shape = model.graph.input[0].type.tensor_type.shape.dim
    if not input_shape or input_shape[-1].dim_value != 200:
        raise ValueError(f"{source.name} has an unexpected input width")
    output_shape = model.graph.output[0].type.tensor_type.shape.dim
    if not output_shape or output_shape[-1].dim_value != expected_units:
        raise ValueError(f"{source.name} has an unexpected output width")

    initializers = list(model.graph.initializer)
    weights: list[dict] = []
    weight_data = bytearray()
    initializer_names: set[str] = set()
    for initializer in initializers:
        if initializer.data_type != onnx.TensorProto.FLOAT or not initializer.raw_data:
            raise ValueError(f"{source.name} has a non-float or empty initializer")
        name = canonical_tensor_name(initializer.name)
        if name in initializer_names:
            raise ValueError(f"{source.name} has duplicate initializer {name}")
        initializer_names.add(name)
        shape = list(initializer.dims)
        weights.append({"name": name, "shape": shape, "dtype": "float32"})
        weight_data.extend(initializer.raw_data)

    def node_name(value: str) -> str:
        return canonical_tensor_name(value)

    output_to_node: dict[str, str] = {}
    graph_nodes: list[dict] = []
    for index, node in enumerate(model.graph.node):
        name = node_name(node.name or (node.output[0] if node.output else f"node_{index}"))
        for output in node.output:
            output_to_node[canonical_tensor_name(output)] = name
        graph_nodes.append({"node": node, "name": name})

    def resolve_input(value: str) -> str:
        normalized = canonical_tensor_name(value)
        if normalized in initializer_names:
            return normalized
        if normalized == canonical_tensor_name(model.graph.input[0].name):
            return "model/Placeholder"
        return output_to_node.get(normalized, normalized)

    nodes = [{
        "name": "model/Placeholder",
        "op": "Placeholder",
        "attr": {
            "dtype": {"type": "DT_FLOAT"},
            "shape": {"shape": {"dim": [{"size": "-1"}, {"size": "200"}]}},
        },
    }]
    for initializer in initializers:
        name = canonical_tensor_name(initializer.name)
        nodes.append({
            "name": name,
            "op": "Const",
            "attr": onnx_tensor_attribute(list(initializer.dims)),
        })

    supported_ops = {"Add", "AddV2", "MatMul", "Mul", "Relu", "Softmax"}
    for item in graph_nodes:
        node = item["node"]
        if node.op_type not in supported_ops:
            raise ValueError(f"{source.name} contains unsupported ONNX op {node.op_type}")
        op = "Add" if node.op_type == "AddV2" else node.op_type
        attrs = {"T": {"type": "DT_FLOAT"}}
        if op == "MatMul":
            attrs.update({"transpose_a": {"b": False}, "transpose_b": {"b": False}})
        elif op == "Softmax":
            attrs["axis"] = {"i": -1}
        nodes.append({
            "name": item["name"],
            "op": op,
            "input": [resolve_input(value) for value in node.input],
            "attr": attrs,
        })

    resolved_output_name = node_name(model.graph.output[0].name)
    if resolved_output_name not in {node["name"] for node in nodes}:
        resolved_output_name = output_to_node.get(resolved_output_name, resolved_output_name)
    if resolved_output_name not in {node["name"] for node in nodes}:
        raise ValueError(f"{source.name} output node cannot be resolved")
    # Essentia's MIREX metadata names this output PartitionedCall.  The ONNX
    # export calls the same tensor model/Softmax; keep the documented public
    # name while retaining the exact ONNX operation and weights.
    output_name = "PartitionedCall"
    for node in nodes:
        if node["name"] == resolved_output_name:
            node["name"] = output_name
            break

    destination.mkdir(parents=True, exist_ok=True)
    (destination / f"{model_id}.json").write_text(
        json.dumps({
            "format": "graph-model",
            "generatedBy": "W4DJ resource preparation from official Essentia ONNX export",
            "convertedBy": "W4DJ",
            "signature": {"outputs": {output_name: {"name": output_name}}},
            "modelTopology": {"node": nodes, "library": {}, "versions": {}},
            "weightsManifest": [{"paths": ["group1-shard1of1.bin"], "weights": weights}],
        }, separators=(",", ":")),
        encoding="utf-8",
    )
    (destination / f"{model_id}.bin").write_bytes(weight_data)


def initializer_bytes(model: onnx.ModelProto, name: str) -> bytes:
    aliases = (name, f"{name}/read:0", f"{name}:0")
    for initializer in model.graph.initializer:
        if initializer.name in aliases:
            if initializer.data_type != onnx.TensorProto.FLOAT:
                raise ValueError(f"{name} is not float32")
            if not initializer.raw_data:
                raise ValueError(f"{name} has no raw tensor data")
            return initializer.raw_data
    raise ValueError(f"missing initializer {name}")


def convert_head(
    source: Path,
    destination: Path,
    model_id: str,
    expected_units: int = 2,
    output_name: str = "model/Softmax",
) -> None:
    if model_id == "mirex":
        convert_mirex(source, destination, model_id, expected_units)
        return
    model = require_onnx().load(source)
    kernel = initializer_bytes(model, "dense/kernel")
    bias = initializer_bytes(model, "dense/bias")
    output_kernel = initializer_bytes(model, "dense_1/kernel")
    output_bias = initializer_bytes(model, "dense_1/bias")
    if len(kernel) != 200 * 100 * 4 or len(bias) != 100 * 4:
        raise ValueError(f"{source.name} has an unexpected hidden layer shape")
    if len(output_bias) % 4 != 0:
        raise ValueError(f"{source.name} has an invalid output bias")
    units = len(output_bias) // 4
    if len(output_kernel) != 100 * units * 4:
        raise ValueError(f"{source.name} has an unexpected output layer shape")
    if units != expected_units:
        raise ValueError(f"{source.name} has {units} outputs; expected {expected_units}")

    destination.mkdir(parents=True, exist_ok=True)
    (destination / f"{model_id}.json").write_text(
        json.dumps(graph_model(units, output_name), separators=(",", ":")), encoding="utf-8"
    )
    # TensorFlow.js consumes tensors in the same order as weightsManifest.
    (destination / f"{model_id}.bin").write_bytes(
        kernel + output_bias + bias + output_kernel
    )


def validate_pair(destination: Path, model_id: str, expected_units: int | None = None,
                  expected_output_name: str | None = None,
                  expected_input_width: int | None = None) -> None:
    """Re-read a generated pair before it can replace an existing resource."""
    model_path = destination / f"{model_id}.json"
    weights_path = destination / f"{model_id}.bin"
    value = json.loads(model_path.read_text(encoding="utf-8"))
    manifests = value.get("weightsManifest") or []
    weights = [weight for manifest in manifests for weight in manifest.get("weights", [])]
    expected_bytes = 0
    for weight in weights:
        dtype_bytes = {"float32": 4, "int32": 4, "bool": 1, "uint8": 1, "int8": 1}.get(weight.get("dtype"))
        if dtype_bytes is None:
            raise ValueError(f"{model_id} has an unsupported weight dtype")
        elements = 1
        for dimension in weight.get("shape", []):
            elements *= int(dimension)
        expected_bytes += elements * dtype_bytes
    if not value.get("modelTopology") or not weights or weights_path.stat().st_size != expected_bytes:
        raise ValueError(f"{model_id} failed the re-read graph/weight validation")
    nodes = {node.get("name") for node in value["modelTopology"].get("node", [])}
    if expected_units is not None:
        output_weights = None
        for marker in ("dense_out/bias", "dense_2/bias", "dense_1/bias", "dense_out/kernel", "dense_2/kernel", "dense_1/kernel"):
            output_weights = next(
                (weight for weight in reversed(weights) if marker in str(weight.get("name", ""))),
                None,
            )
            if output_weights is not None:
                break
        output_shape = output_weights.get("shape") if output_weights else None
        output_matches = output_shape == [expected_units]
        if not output_matches and isinstance(output_shape, list) and output_shape:
            output_matches = output_shape[-1] == expected_units
        if not output_matches:
            output_node = next(
                (node for node in value["modelTopology"].get("node", [])
                 if node.get("name") == expected_output_name),
                None,
            )
            dimensions = (((output_node or {}).get("attr", {}).get("_output_shapes", {})
                           .get("list", {}).get("shape", [{}])[-1]).get("dim", []))
            last_size = dimensions[-1].get("size") if dimensions else None
            if str(last_size) != str(expected_units):
                raise ValueError(f"{model_id} has an unexpected output width")
    if expected_output_name is not None and expected_output_name not in nodes:
        raise ValueError(f"{model_id} is missing output node {expected_output_name}")
    if expected_input_width is not None:
        placeholders = [
            node for node in value["modelTopology"].get("node", [])
            if node.get("op") == "Placeholder"
        ]
        if not any(
            str(dim.get("size")) == str(expected_input_width)
            for node in placeholders
            for dim in (((node.get("attr") or {}).get("shape") or {}).get("shape") or {}).get("dim", [])
            if isinstance(dim, dict)
        ):
            raise ValueError(f"{model_id} has an unexpected input width")


def validate_output_set(destination: Path) -> None:
    validate_pair(destination, "musicnn_embedding")
    for model_id in HEAD_IDS:
        if (destination / f"{model_id}.json").is_file():
            validate_pair(destination, model_id, 2)
    for model_id, _filename, units, output_name in EMOTION_HEADS:
        if (destination / f"{model_id}.json").is_file():
            validate_pair(destination, model_id, units, output_name)
    for model_id, output_name, units, _index in DISCOGS_HEADS:
        if (destination / f"{model_id}.json").is_file():
            validate_pair(destination, model_id, units, output_name)
    for model_id, _source_name, output_name, units, input_width in DISCOGS_EXTRA_HEADS:
        if (destination / f"{model_id}.json").is_file():
            validate_pair(destination, model_id, units, output_name, input_width)


def normalize_discogs_pair(source: Path, destination: Path, model_id: str,
                           output_name: str, expected_units: int, output_index: int) -> None:
    """Normalize one official converter directory into W4DJ's flat pair."""
    model_path = source / "model.json"
    if not model_path.is_file():
        raise ValueError(f"{source} has no model.json")
    value = json.loads(model_path.read_text(encoding="utf-8"))
    if any(node.get("op") == "PartitionedCall"
           for node in (value.get("modelTopology") or {}).get("node", [])):
        inline_tfjs_function_graph(value, output_name, output_index)
    shards = sorted(source.glob("*.bin"))
    if not shards:
        raise ValueError(f"{source} has no TensorFlow.js weight shard")
    weights = [weight for manifest in value.get("weightsManifest", [])
               for weight in manifest.get("weights", [])]
    value["weightsManifest"] = [{"paths": [f"{model_id}.bin"], "weights": weights}]
    destination.mkdir(parents=True, exist_ok=True)
    (destination / f"{model_id}.json").write_text(
        json.dumps(value, separators=(",", ":")), encoding="utf-8"
    )
    with (destination / f"{model_id}.bin").open("wb") as output:
        for shard in shards:
            with shard.open("rb") as input_file:
                shutil.copyfileobj(input_file, output)
    validate_pair(destination, model_id, expected_units, output_name)


def _normalise_legacy_embedding_json(value: object) -> object:
    """Map the old duplicate's manifest filename to the canonical name."""
    if isinstance(value, dict):
        return {key: _normalise_legacy_embedding_json(item) for key, item in value.items()}
    if isinstance(value, list):
        return [_normalise_legacy_embedding_json(item) for item in value]
    if value == f"{LEGACY_DISCOGS_EMBEDDING_ID}.bin":
        return "discogs_effnet_embedding.bin"
    return value


def legacy_embedding_json_matches_canonical(legacy: Path, canonical: Path) -> bool:
    """Recognise an older exact duplicate whose only difference is its shard name."""
    try:
        legacy_value = json.loads(legacy.read_text(encoding="utf-8"))
        canonical_value = json.loads(canonical.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError):
        return False
    return _normalise_legacy_embedding_json(legacy_value) == canonical_value


def normalize_discogs_head(source: Path, destination: Path, model_id: str,
                           source_name: str, output_name: str, expected_units: int,
                           expected_input_width: int) -> None:
    """Normalize one official classification head without changing its graph."""
    model_path = source / "model.json"
    if not model_path.is_file():
        raise ValueError(f"{source} has no model.json")
    value = json.loads(model_path.read_text(encoding="utf-8"))
    nodes = {node.get("name") for node in (value.get("modelTopology") or {}).get("node", [])}
    if output_name not in nodes:
        raise ValueError(f"{source_name} graph is missing {output_name}")
    shards = sorted(source.glob("*.bin"))
    if not shards:
        raise ValueError(f"{source} has no TensorFlow.js weight shard")
    weights = [weight for manifest in value.get("weightsManifest", [])
               for weight in manifest.get("weights", [])]
    value["weightsManifest"] = [{"paths": [f"{model_id}.bin"], "weights": weights}]
    destination.mkdir(parents=True, exist_ok=True)
    (destination / f"{model_id}.json").write_text(
        json.dumps(value, separators=(",", ":")), encoding="utf-8"
    )
    with (destination / f"{model_id}.bin").open("wb") as output:
        for shard in shards:
            with shard.open("rb") as input_file:
                shutil.copyfileobj(input_file, output)
    validate_pair(destination, model_id, expected_units, output_name, expected_input_width)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--embedding-json", type=Path, required=True)
    parser.add_argument("--embedding-bin", type=Path, required=True)
    parser.add_argument("--onnx-dir", type=Path, required=True)
    parser.add_argument(
        "--emotion-onnx-dir",
        type=Path,
        help="Optional local ONNX exports for emoMusic, MuSe and MIREX; missing files are skipped.",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--discogs-embedding-dir", type=Path)
    parser.add_argument("--discogs-genre-dir", type=Path)
    parser.add_argument(
        "--discogs-heads-dir",
        type=Path,
        help="Directory containing the five official Discogs head converter outputs.",
    )
    args = parser.parse_args()

    args.output.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix=f".{args.output.name}.staging-", dir=args.output.parent))
    try:
        shutil.copyfile(args.embedding_json, staging / "musicnn_embedding.json")
        shutil.copyfile(args.embedding_bin, staging / "musicnn_embedding.bin")
        for model_id in HEAD_IDS:
            source = args.onnx_dir / f"{model_id}-msd-musicnn-1.onnx"
            if source.is_file():
                convert_head(source, staging, model_id)
        if args.emotion_onnx_dir:
            for model_id, filename, units, output_name in EMOTION_HEADS:
                source = args.emotion_onnx_dir / filename
                if source.is_file():
                    convert_head(source, staging, model_id, units, output_name)
        if args.discogs_embedding_dir:
            normalize_discogs_pair(args.discogs_embedding_dir, staging, *DISCOGS_HEADS[0])
        if args.discogs_genre_dir:
            normalize_discogs_pair(args.discogs_genre_dir, staging, *DISCOGS_HEADS[1])
        if args.discogs_heads_dir:
            for model_id, source_name, output_name, units, input_width in DISCOGS_EXTRA_HEADS:
                source = args.discogs_heads_dir / source_name
                if not source.is_dir():
                    raise ValueError(f"missing required Discogs head directory: {source}")
                normalize_discogs_head(
                    source, staging, model_id, source_name, output_name, units, input_width
                )
        validate_output_set(staging)
        args.output.mkdir(parents=True, exist_ok=True)
        canonical_json = staging / "discogs_effnet_embedding.json"
        canonical_bin = staging / "discogs_effnet_embedding.bin"
        legacy_json = args.output / f"{LEGACY_DISCOGS_EMBEDDING_ID}.json"
        legacy_bin = args.output / f"{LEGACY_DISCOGS_EMBEDDING_ID}.bin"
        # Do not remove a user-provided legacy model.  Only clean the old
        # duplicate pair when both files are byte-for-byte copies of the new
        # canonical pair.
        if (
            canonical_json.is_file()
            and canonical_bin.is_file()
            and legacy_json.is_file()
            and legacy_bin.is_file()
            and legacy_embedding_json_matches_canonical(legacy_json, canonical_json)
            and legacy_bin.read_bytes() == canonical_bin.read_bytes()
        ):
            legacy_json.unlink()
            legacy_bin.unlink()
        for staged_file in staging.iterdir():
            os.replace(staged_file, args.output / staged_file.name)
    finally:
        shutil.rmtree(staging, ignore_errors=True)


if __name__ == "__main__":
    main()
